// SPDX-License-Identifier: GPL-3.0-or-later
//! Authenticated Android .lumebackup files. Only fully verified staging can be restored.
use super::{store::open, wire, *};
use crate::{
    catalog::CatalogChannel,
    preferences::{PreferenceStore, Preferences},
};
use anyhow::{anyhow, bail, Result};
use ring::{
    aead, pbkdf2,
    rand::{SecureRandom, SystemRandom},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    thread,
};
const MAX: u64 = 2 * 1024 * 1024 * 1024;
const CHUNK: usize = 256 * 1024;
const MAX_JSON: u64 = 16 * 1024 * 1024;
const MAX_ROW: usize = 2 * 1024 * 1024;

fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
fn create(path: &Path) -> Result<File> {
    let f = File::create(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        f.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok(f)
}
fn password(value: &str) -> Result<()> {
    if !(8..=128).contains(&value.encode_utf16().count()) {
        bail!("Usa una password da 8 a 128 caratteri.");
    }
    Ok(())
}
fn invalid() -> anyhow::Error {
    anyhow!("Password errata o backup danneggiato.")
}
fn key(password: &str, header: &[u8; 32]) -> Result<aead::LessSafeKey> {
    let mut raw = [0; 32];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        NonZeroU32::new(600_000).unwrap(),
        &header[8..24],
        password.as_bytes(),
        &mut raw,
    );
    let result = aead::UnboundKey::new(&aead::AES_256_GCM, &raw).map_err(|_| invalid());
    raw.fill(0);
    Ok(aead::LessSafeKey::new(result?))
}
fn nonce(header: &[u8; 32], index: u32) -> aead::Nonce {
    let mut n = [0; 12];
    n[..8].copy_from_slice(&header[24..]);
    n[8..].copy_from_slice(&index.to_be_bytes());
    aead::Nonce::assume_unique_for_key(n)
}
fn aad(header: &[u8; 32], index: u32, len: u32) -> Vec<u8> {
    let mut a = header.to_vec();
    a.extend_from_slice(&index.to_be_bytes());
    a.extend_from_slice(&len.to_be_bytes());
    a
}
fn encrypt(
    input: &mut dyn Read,
    output: &mut dyn Write,
    password: &str,
    check: &dyn Fn() -> Result<()>,
) -> Result<()> {
    self::password(password)?;
    let mut header = [0; 32];
    header[..8].copy_from_slice(b"LUMEBK01");
    SystemRandom::new()
        .fill(&mut header[8..])
        .map_err(|_| invalid())?;
    let key = key(password, &header)?;
    output.write_all(&header)?;
    let mut index = 0;
    let mut total = 0;
    loop {
        check()?;
        let mut bytes = vec![0; CHUNK];
        let mut length = 0;
        while length < CHUNK {
            let n = input.read(&mut bytes[length..])?;
            if n == 0 {
                break;
            }
            length += n;
        }
        total += length as u64;
        if total > MAX {
            bail!("Il backup supera il limite di 2 GiB.");
        }
        bytes.truncate(length);
        key.seal_in_place_append_tag(
            nonce(&header, index),
            aead::Aad::from(aad(&header, index, length as u32)),
            &mut bytes,
        )
        .map_err(|_| invalid())?;
        output.write_all(&(length as u32).to_be_bytes())?;
        output.write_all(&bytes)?;
        bytes.fill(0);
        if length == 0 {
            break;
        }
        index += 1;
    }
    Ok(())
}
fn decrypt(
    input: &mut dyn Read,
    output: &mut dyn Write,
    password: &str,
    check: &dyn Fn() -> Result<()>,
) -> Result<()> {
    self::password(password)?;
    let mut header = [0; 32];
    input.read_exact(&mut header).map_err(|_| invalid())?;
    if &header[..8] != b"LUMEBK01" {
        bail!("Questo file non è un backup Lume supportato.");
    }
    let key = key(password, &header)?;
    let mut index = 0;
    let mut total = 0;
    loop {
        check()?;
        let mut raw = [0; 4];
        input.read_exact(&mut raw).map_err(|_| invalid())?;
        let len = u32::from_be_bytes(raw) as usize;
        if len > CHUNK {
            return Err(invalid());
        }
        total += len as u64 + 20;
        if total > MAX + 1024 * 1024 {
            bail!("Il backup supera il limite di 2 GiB.");
        }
        let mut bytes = vec![0; len + 16];
        input.read_exact(&mut bytes).map_err(|_| invalid())?;
        let plain = key
            .open_in_place(
                nonce(&header, index),
                aead::Aad::from(aad(&header, index, len as u32)),
                &mut bytes,
            )
            .map_err(|_| invalid())?;
        if len == 0 {
            if input.read(&mut [0])? != 0 {
                return Err(invalid());
            }
            break;
        }
        output.write_all(plain)?;
        bytes.fill(0);
        index += 1;
    }
    Ok(())
}
fn line(output: &mut dyn Write, value: &Value, total: &mut u64) -> Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    if bytes.len() > MAX_ROW {
        bail!("Il backup contiene una voce troppo grande.");
    }
    bytes.push(b'\n');
    *total += bytes.len() as u64;
    if *total > MAX {
        bail!("Il backup espanso supera il limite di 2 GiB.");
    }
    output.write_all(&bytes)?;
    Ok(())
}
struct Expanded<'a, R> {
    input: R,
    total: &'a mut u64,
}
impl<R: Read> Read for Expanded<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let limit = buffer
            .len()
            .min(MAX.saturating_sub(*self.total).saturating_add(1) as usize);
        let n = self.input.read(&mut buffer[..limit])?;
        *self.total += n as u64;
        if *self.total > MAX {
            return Err(std::io::Error::other(
                "Il backup espanso supera il limite di 2 GiB.",
            ));
        }
        Ok(n)
    }
}
fn read_line(input: &mut dyn BufRead) -> Result<Option<Value>> {
    let mut bytes = Vec::new();
    let n = input
        .take(MAX_ROW as u64 + 2)
        .read_until(b'\n', &mut bytes)?;
    if n == 0 {
        return Ok(None);
    }
    if bytes.len() > MAX_ROW + 1 || bytes == b"\n" {
        bail!("Voce del backup non valida o troppo grande.");
    }
    Ok(Some(serde_json::from_slice(&bytes)?))
}
struct Job {
    id: String,
    directory: PathBuf,
    state: Mutex<Value>,
    cancelled: AtomicBool,
    busy: AtomicBool,
}
impl Job {
    fn check(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Acquire) {
            bail!("Operazione annullata.");
        }
        Ok(())
    }
    fn phase(&self, phase: &str) {
        self.state.lock().unwrap()["phase"] = json!(phase);
    }
    fn fail(&self, error: anyhow::Error) {
        let mut s = self.state.lock().unwrap();
        s["phase"] = json!(if self.cancelled.load(Ordering::Acquire) {
            "cancelled"
        } else {
            "error"
        });
        s["error"] = json!(error.to_string());
        let _ = fs::remove_dir_all(&self.directory);
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}
pub struct Backups {
    directory: PathBuf,
    jobs: Mutex<BTreeMap<String, Arc<Job>>>,
}
impl Backups {
    pub fn new(root: &Path) -> Result<Arc<Self>> {
        let directory = root.join("personal/backup-work");
        private_dir(&directory)?;
        for f in fs::read_dir(&directory)? {
            let f = f?;
            if f.file_type()?.is_dir() {
                let _ = fs::remove_dir_all(f.path());
            }
        }
        Ok(Arc::new(Self {
            directory,
            jobs: Mutex::new(BTreeMap::new()),
        }))
    }
    fn new_job(&self, phase: &str) -> Result<Arc<Job>> {
        let mut jobs = self.jobs.lock().unwrap();
        jobs.retain(|_, j| j.busy.load(Ordering::Acquire) || !j.cancelled.load(Ordering::Acquire));
        if jobs.len() >= 4 {
            bail!("Chiudi un’anteprima prima di creare un altro backup.");
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        let directory = self.directory.join(&id);
        private_dir(&directory)?;
        let job = Arc::new(Job {
            id: id.clone(),
            directory,
            state: Mutex::new(json!({"id":id,"phase":phase,"error":""})),
            cancelled: AtomicBool::new(false),
            busy: AtomicBool::new(false),
        });
        jobs.insert(id, job.clone());
        Ok(job)
    }
    fn get(&self, id: &str) -> Result<Arc<Job>> {
        self.jobs
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow!("Anteprima non disponibile. Seleziona di nuovo il backup."))
    }
    pub fn status(&self, id: &str) -> Result<Value> {
        Ok(self.get(id)?.state.lock().unwrap().clone())
    }
    pub fn input(&self) -> Result<Value> {
        let j = self.new_job("upload")?;
        self.status(&j.id)
    }
    pub fn upload(&self, id: &str, input: &mut dyn Read) -> Result<Value> {
        let j = self.get(id)?;
        if j.state.lock().unwrap()["phase"] != "upload" || j.busy.swap(true, Ordering::AcqRel) {
            bail!("Caricamento non disponibile.");
        }
        let result = (|| {
            let mut f = create(&j.directory.join("input.lumebackup"))?;
            let n = std::io::copy(&mut input.take(MAX + 1024 * 1024 + 1), &mut f)?;
            j.check()?;
            if n > MAX + 1024 * 1024 {
                bail!("Il backup supera il limite di 2 GiB.");
            }
            f.sync_all()?;
            j.phase("uploaded");
            Ok(())
        })();
        j.busy.store(false, Ordering::Release);
        if let Err(e) = result {
            j.fail(e);
        }
        self.status(id)
    }
    pub fn prepare(&self, id: &str, mut password: String) -> Result<Value> {
        self::password(&password)?;
        let j = self.get(id)?;
        if j.state.lock().unwrap()["phase"] != "uploaded" || j.busy.swap(true, Ordering::AcqRel) {
            bail!("Carica di nuovo il backup.");
        }
        j.phase("verifying");
        thread::spawn(move || {
            let result = prepare(&j, &password);
            unsafe { password.as_bytes_mut().fill(0) };
            if let Err(e) = result {
                j.fail(e);
            }
            let _ = fs::remove_file(j.directory.join("input.lumebackup"));
            let _ = fs::remove_file(j.directory.join("archive.zip"));
            j.busy.store(false, Ordering::Release);
        });
        self.status(id)
    }
    pub fn export(
        &self,
        library: Arc<Library>,
        preferences: Preferences,
        builtins: Vec<CatalogChannel>,
        mut password: String,
    ) -> Result<Value> {
        self::password(&password)?;
        let j = self.new_job("exporting")?;
        j.busy.store(true, Ordering::Release);
        let id = j.id.clone();
        thread::spawn(move || {
            let result = export(&j, &library, &preferences, &builtins, &password);
            unsafe { password.as_bytes_mut().fill(0) };
            if let Err(e) = result {
                j.fail(e);
            }
            let _ = fs::remove_file(j.directory.join("archive.zip"));
            j.busy.store(false, Ordering::Release);
        });
        self.status(&id)
    }
    pub fn cancel(&self, id: &str) -> Result<Value> {
        let j = self.get(id)?;
        j.cancelled.store(true, Ordering::Release);
        if !j.busy.load(Ordering::Acquire) {
            self.jobs.lock().unwrap().remove(id);
        }
        Ok(json!({"cancelled":true}))
    }
    pub fn download(&self, id: &str) -> Result<File> {
        let j = self.get(id)?;
        j.check()?;
        if j.state.lock().unwrap()["phase"] != "exported" {
            bail!("Backup non pronto.");
        }
        Ok(File::open(j.directory.join("export.lumebackup"))?)
    }
    pub fn restore(
        &self,
        id: &str,
        selection: &Value,
        library: &Library,
        prefs: &PreferenceStore,
        builtins: &[CatalogChannel],
    ) -> Result<Value> {
        let j = self.get(id)?;
        j.check()?;
        if j.state.lock().unwrap()["phase"] != "ready" || j.busy.swap(true, Ordering::AcqRel) {
            bail!("L’anteprima non è pronta.");
        }
        let result = restore(&j, selection, library, prefs, builtins);
        j.busy.store(false, Ordering::Release);
        if let Ok(value) = &result {
            let mut state = j.state.lock().unwrap();
            state["phase"] = json!("restored");
            state["result"] = json!({"playlists":value["playlists"],"kept":value["kept"],"favorites":value["favorites"],"watch":value["watch"],"skipped":value["skipped"]});
        }
        result
    }
}
fn export(
    j: &Job,
    library: &Library,
    prefs: &Preferences,
    builtins: &[CatalogChannel],
    password: &str,
) -> Result<()> {
    use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};
    let _operation = library.operation.lock().unwrap();
    let sources = library.sources.lock().unwrap().clone();
    let mut zip = ZipWriter::new(create(&j.directory.join("archive.zip"))?);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o600);
    let mut expanded = 0_u64;
    let mut owners = serde_json::Map::new();
    let mut references = BTreeMap::new();
    let mut favorites = Vec::new();
    let mut wanted = prefs.favorites.iter().cloned().collect::<BTreeSet<_>>();
    if let Some(rows) = prefs.channel_layout["order"].as_array() {
        wanted.extend(rows.iter().filter_map(Value::as_str).map(str::to_owned));
    }
    if let Some(rows) = prefs.channel_layout["hidden"].as_array() {
        wanted.extend(
            rows.iter()
                .filter_map(|v| v["id"].as_str())
                .map(str::to_owned),
        );
    }

    for c in builtins {
        references.insert(
            c.id.clone(),
            (
                wire::builtin_id(c),
                if c.kind == crate::model::MediaKind::Radio {
                    "builtin_radio"
                } else {
                    "builtin_tv"
                }
                .to_string(),
            ),
        );
    }
    for s in sources.values() {
        j.check()?;
        j.state.lock().unwrap()["message"] = json!(format!("Salvo {}…", s.name));
        zip.start_file(format!("lists/{}.jsonl", s.id), options)?;
        line(&mut zip, &wire::source(s), &mut expanded)?;
        let db = open(&library.path(&s.id)?)?;
        let mut rows =
            db.prepare("SELECT id,payload,active,seen,position FROM records ORDER BY position,id")?;
        let mut rows = rows.query([])?;
        while let Some(r) = rows.next()? {
            j.check()?;
            let id: String = r.get(0)?;
            let mut e: Entry = serde_json::from_str(&r.get::<_, String>(1)?)?;
            e.id = id.clone();
            e.position = r.get(4)?;
            e.content = video::classify(&e, &s.settings).into();
            let key = wire::channel_id(&id);
            if wanted.contains(&id) {
                references.insert(id.clone(), (key.clone(), s.id.clone()));
            }
            line(
                &mut zip,
                &json!({"id":key,"desktopId":id,"entry":wire::entry_for(&e,s),"active":r.get::<_,bool>(2)?,"seen":r.get::<_,u64>(3)?}),
                &mut expanded,
            )?;
        }
    }
    for id in &prefs.favorites {
        if let Some((key, source)) = references.get(id) {
            favorites.push(key.clone());
            owners.insert(key.clone(), json!(source));
        }
    }
    let order = prefs.channel_layout["order"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().and_then(|id| references.get(id)))
        .map(|(id, source)| {
            owners.insert(id.clone(), json!(source));
            json!(id)
        })
        .collect::<Vec<_>>();
    let hidden = prefs.channel_layout["hidden"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let (id, source) = references.get(row["id"].as_str()?)?;
            owners.insert(id.clone(), json!(source));
            let mut row = row.clone();
            row["id"] = json!(id);
            row["source"] = json!(source);
            Some(row)
        })
        .collect::<Vec<_>>();
    let layout = json!({"version":1,"order":order,"hidden":hidden,"owners":{}});
    let db = library.watch_db()?;
    let mut series = Vec::new();
    let mut saved =
        db.prepare("SELECT source,key FROM video_saved ORDER BY source,key LIMIT 5000")?;
    for row in saved.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (s, k) = row?;
        if !sources.contains_key(&s) {
            continue;
        }
        if let Some(series_key) = k.strip_prefix("s:") {
            series.push(format!("{s}:{series_key}"));
        } else if let Some(id) = k.strip_prefix("i:") {
            let exists: bool = open(&library.path(&s)?)?.query_row(
                "SELECT EXISTS(SELECT 1 FROM records WHERE id=?)",
                [id],
                |r| r.get(0),
            )?;
            if !exists {
                continue;
            }
            let key = wire::channel_id(id);
            if !favorites.contains(&key) {
                favorites.push(key.clone());
                owners.insert(key, json!(s));
            }
        }
    }
    let mut watches = Vec::new();
    let mut rows=db.prepare("SELECT source,channel,mode,event,position,duration,completed,hidden,updated FROM watch ORDER BY updated DESC LIMIT 5000")?;
    for row in rows.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, f64>(4)?,
            r.get::<_, f64>(5)?,
            r.get::<_, bool>(6)?,
            r.get::<_, bool>(7)?,
            r.get::<_, u64>(8)?,
        ))
    })? {
        let (s, id, mode, event, pos, dur, completed, hidden, updated) = row?;
        if sources.contains_key(&s) {
            watches.push(json!({"source":s,"id":wire::channel_id(&id),"desktopId":id,"mode":mode.to_uppercase(),"event":event,"position":((pos*1000.0) as u64).min(31_536_000_000),"duration":((dur*1000.0)as u64).min(31_536_000_000),"completed":completed,"hidden":hidden,"updated":updated}));
        }
    }
    let metadata = library.video_backup_choices(&sources, &mut owners)?;
    let manifest = json!({"format":"lumetv-backup","version":1,"created":crate::report::now_ms(),"sources":sources.keys().collect::<Vec<_>>(),"personal":{"favorites":favorites,"series":series,"owners":owners,"layout":layout,"metadata":metadata},"preferences":{"data_saver":false,"audio_language":prefs.audio_language,"subtitle_mode":prefs.subtitle_mode.to_uppercase(),"subtitle_language":prefs.subtitle_language,"zap_favorites_only":prefs.zap_favorites_only},"watch":watches,"desktop":{"preferences":prefs,"source_names":sources.iter().map(|(id,s)|(id.clone(),json!(s.name))).collect::<serde_json::Map<_,_>>(),"guide_mappings":sources.iter().map(|(id,s)|(id.clone(),json!(s.settings.guide.mappings))).collect::<serde_json::Map<_,_>>()}});
    let bytes = serde_json::to_vec(&manifest)?;
    if bytes.len() as u64 > MAX_JSON || expanded + bytes.len() as u64 > MAX {
        bail!("Le personalizzazioni superano il limite del backup.");
    }
    zip.start_file("manifest.json", options)?;
    zip.write_all(&bytes)?;
    zip.finish()?.sync_all()?;
    j.phase("encrypting");
    let mut output = create(&j.directory.join("export.lumebackup"))?;
    encrypt(
        &mut File::open(j.directory.join("archive.zip"))?,
        &mut output,
        password,
        &|| j.check(),
    )?;
    output.sync_all()?;
    j.check()?;
    *j.state.lock().unwrap() = json!({"id":j.id,"phase":"exported","url":format!("/backup/{}/Lume.lumebackup",j.id),"bytes":output.metadata()?.len(),"error":""});
    Ok(())
}
fn prepare(j: &Job, password: &str) -> Result<()> {
    let mut output = create(&j.directory.join("archive.zip"))?;
    decrypt(
        &mut File::open(j.directory.join("input.lumebackup"))?,
        &mut output,
        password,
        &|| j.check(),
    )?;
    drop(output);
    j.phase("reading");
    let mut archive = zip::ZipArchive::new(File::open(j.directory.join("archive.zip"))?)?;
    if archive.len() > 11 {
        bail!("Troppe liste nel backup.");
    }
    let registry = open(&j.directory.join("registry.sqlite"))?;
    registry.execute_batch("CREATE TABLE refs(id TEXT PRIMARY KEY,desktop TEXT UNIQUE,source TEXT,content TEXT,kind TEXT,active INTEGER)")?;
    let mut seen = BTreeSet::new();
    let mut sources = BTreeMap::new();
    let mut manifest = None;
    let mut expanded = 0u64;
    let mut actual_expanded = 0u64;
    let mut staged_bytes = 0u64;
    let mut total = 0usize;
    for i in 0..archive.len() {
        j.check()?;
        let entry = archive.by_index(i)?;
        let name = entry.name()?.into_owned();
        if entry.is_dir()
            || !seen.insert(name.clone())
            || entry
                .unix_mode()
                .is_some_and(|m| m & 0o170000 != 0 && m & 0o170000 != 0o100000)
        {
            bail!("Struttura del backup non valida.");
        }
        expanded = expanded.checked_add(entry.size()).ok_or_else(invalid)?;
        if expanded > MAX {
            bail!("Il backup espanso supera il limite di 2 GiB.");
        }
        let entry = Expanded {
            input: entry,
            total: &mut actual_expanded,
        };
        if name == "manifest.json" {
            let mut bytes = Vec::new();
            entry.take(MAX_JSON + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > MAX_JSON {
                bail!("Manifest del backup troppo grande.");
            }
            manifest = Some(serde_json::from_slice::<Value>(&bytes)?);
            continue;
        }
        let id = name
            .strip_prefix("lists/")
            .and_then(|v| v.strip_suffix(".jsonl"))
            .filter(|v| Library::valid_source(v))
            .ok_or_else(|| anyhow!("File non previsto nel backup."))?;
        let mut reader = BufReader::with_capacity(64 * 1024, entry);
        let header = read_line(&mut reader)?.ok_or_else(invalid)?;
        let mut source = wire::read_source(&header)?;
        if source.id != id {
            bail!("Identificativo della lista non valido.");
        }
        let db = open(&j.directory.join(format!("{id}.sqlite")))?;
        db.execute_batch("PRAGMA max_page_count=262144; CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT);CREATE TABLE records(id TEXT NOT NULL,name TEXT NOT NULL,number INTEGER,kind TEXT NOT NULL,content TEXT NOT NULL,position INTEGER NOT NULL,payload TEXT NOT NULL,identity1 TEXT,identity2 TEXT,identity3 TEXT,seen INTEGER NOT NULL,active INTEGER NOT NULL); CREATE TABLE groups(name TEXT NOT NULL,position INTEGER NOT NULL,PRIMARY KEY(name,position));BEGIN IMMEDIATE;")?;
        let expected = source.report.count;
        let mut active = 0;
        let mut archived = 0;
        while let Some(row) = read_line(&mut reader)? {
            j.check()?;
            let key = row["id"]
                .as_str()
                .filter(|v| v.len() == 24 && v.bytes().all(|b| b.is_ascii_hexdigit()))
                .ok_or_else(invalid)?;
            let desktop = row["desktopId"]
                .as_str()
                .filter(|v| {
                    v.starts_with("personal_")
                        && crate::catalog::valid_id(v)
                        && wire::channel_id(v) == key
                })
                .map(str::to_owned)
                .unwrap_or_else(|| format!("personal_{key}"));
            let entry = wire::read_entry(&row["entry"], &header, &source.settings, &desktop)?;
            let enabled = row["active"].as_bool().ok_or_else(invalid)?;
            let date = row["seen"].as_u64().ok_or_else(invalid)?;
            if enabled {
                active += 1;
                total += 1;
            } else {
                archived += 1;
            }
            if active > MAX_ENTRIES || archived > 5000 || total > MAX_TOTAL {
                bail!("Il backup supera i limiti della libreria.");
            }
            let kind = if entry.kind == crate::model::MediaKind::Radio {
                "radio"
            } else {
                "tv"
            };
            let identity = |v: &str| wire::hash(&format!("{kind}|{}|{v}", entry.content));
            registry.execute(
                "INSERT INTO refs VALUES(?,?,?,?,?,?)",
                params![key, desktop, id, entry.content, kind, enabled],
            )?;
            db.execute(
                "INSERT INTO records VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    desktop,
                    entry.name,
                    entry.number,
                    kind,
                    entry.content,
                    entry.position,
                    serde_json::to_string(&entry)?,
                    (!entry.metadata.tvg_id.is_empty()).then(|| identity(&entry.metadata.tvg_id)),
                    identity(&entry.url),
                    identity(&format!(
                        "{}|{}",
                        entry.name.to_lowercase(),
                        entry.groups.join("|")
                    )),
                    date,
                    enabled
                ],
            )?;
            if enabled {
                for g in &entry.groups {
                    db.execute(
                        "INSERT OR IGNORE INTO groups VALUES(?,?)",
                        params![g, entry.position],
                    )?;
                }
                source.report.vod += usize::from(entry.content == "vod");
                source.report.radio += usize::from(kind == "radio");
            }
        }
        if active != expected {
            bail!("Il numero di voci della lista non corrisponde al backup.");
        }
        db.execute_batch("CREATE UNIQUE INDEX by_id ON records(id);CREATE INDEX by_position ON records(active,kind,content,position);CREATE INDEX match1 ON records(identity1);CREATE INDEX match2 ON records(identity2);CREATE INDEX match3 ON records(identity3);COMMIT;")?;
        db.execute(
            "INSERT INTO metadata VALUES('source_snapshot',?)",
            [&source.snapshot],
        )?;
        video::ensure_content(&db, &source.settings)?;
        video::index(&db, &source.settings.content, source.updated_at, &|| {
            j.check()
        })?;
        staged_bytes += fs::metadata(j.directory.join(format!("{id}.sqlite")))?.len();
        if staged_bytes > MAX_DISK {
            bail!("Le liste preparate superano il limite di 2 GiB.");
        }
        sources.insert(id.to_owned(), source);
    }
    let manifest = manifest.ok_or_else(|| anyhow!("Il backup è incompleto."))?;
    validate_manifest(&manifest, &sources, &registry)?;
    for source in sources.values_mut() {
        if let Some(name) = manifest["desktop"]["source_names"][&source.id]
            .as_str()
            .filter(|s| {
                !s.trim().is_empty() && s.chars().count() <= 160 && !s.chars().any(char::is_control)
            })
        {
            source.name = name.into();
        }
        if let Some(mappings) = manifest["desktop"]["guide_mappings"].get(&source.id) {
            source.settings.guide.mappings = serde_json::from_value(mappings.clone())?;
            source.settings.validate()?;
        }
    }

    create(&j.directory.join("manifest.json"))?.write_all(&serde_json::to_vec(&manifest)?)?;
    create(&j.directory.join("sources.json"))?.write_all(&serde_json::to_vec(&sources)?)?;
    j.check()?;
    *j.state.lock().unwrap() = json!({"id":j.id,"phase":"ready","created":manifest["created"],"sources":sources.values().map(Source::public).collect::<Vec<_>>(),"favorites":manifest["personal"]["favorites"].as_array().map_or(0,Vec::len),"series":manifest["personal"]["series"].as_array().map_or(0,Vec::len),"watch":manifest["watch"].as_array().map_or(0,Vec::len),"error":""});
    Ok(())
}
fn strings(value: &Value, max: usize) -> Result<BTreeSet<String>> {
    let rows = value.as_array().ok_or_else(invalid)?;
    if rows.len() > max {
        return Err(invalid());
    }
    let mut seen = BTreeSet::new();
    for row in rows {
        let s = row.as_str().ok_or_else(invalid)?;
        if !seen.insert(s.to_owned()) {
            return Err(invalid());
        }
    }
    Ok(seen)
}
fn validate_manifest(
    m: &Value,
    sources: &BTreeMap<String, Source>,
    registry: &Connection,
) -> Result<()> {
    if m["format"] != "lumetv-backup"
        || m["version"] != 1
        || m["created"].as_u64().is_none_or(|v| v > 4_102_444_800_000)
        || strings(&m["sources"], 10)? != sources.keys().cloned().collect()
    {
        bail!("Versione o contenuto del backup non supportato.");
    }
    let p = &m["personal"];
    let mut refs = strings(&p["favorites"], 200_000)?;
    refs.extend(strings(&p["layout"]["order"], 200_000)?);
    if p["layout"]["version"] != 1 {
        return Err(invalid());
    }
    let hidden = p["layout"]["hidden"].as_array().ok_or_else(invalid)?;
    if hidden.len() > 200_000 {
        return Err(invalid());
    }
    let mut hidden_ids = BTreeSet::new();
    for row in hidden {
        let id = row["id"].as_str().ok_or_else(invalid)?;
        if !hidden_ids.insert(id)
            || row["name"]
                .as_str()
                .is_none_or(|s| s.encode_utf16().count() > 160)
            || !matches!(row["kind"].as_str(), Some("TV" | "RADIO"))
            || row["source"] != p["owners"][id]
        {
            return Err(invalid());
        }
        refs.insert(id.into());
    }
    if let Some(rows) = p["metadata"].as_array() {
        if rows.len() > 5000 {
            return Err(invalid());
        }
        let mut seen = BTreeSet::new();
        for r in rows {
            let key = r["key"].as_str().ok_or_else(invalid)?;
            if !sources.contains_key(r["source"].as_str().unwrap_or(""))
                || key.len() > 162
                || !(key.starts_with("i:") || key.starts_with("s:"))
                || !seen.insert((r["source"].to_string(), key))
                || !matches!(r["type"].as_str(), Some("MOVIE" | "TV"))
                || r["excluded"].as_bool().is_none()
                || r["excluded"] == true && !r["id"].is_null()
                || r["excluded"] == false
                    && r["id"]
                        .as_u64()
                        .is_none_or(|v| v == 0 || v > u32::MAX as u64)
                || r["fingerprint"]
                    .as_str()
                    .is_none_or(|v| v.len() != 64 || !v.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                return Err(invalid());
            }
            if let Some(id) = key.strip_prefix("i:") {
                refs.insert(id.to_owned());
            }
        }
    }
    let owners = p["owners"].as_object().ok_or_else(invalid)?;
    if owners.len() != refs.len() || refs.len() > 200_000 {
        return Err(invalid());
    }
    for id in refs {
        if id.len() != 24 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid());
        }
        let owner = owners
            .get(&id)
            .and_then(Value::as_str)
            .ok_or_else(invalid)?;
        if !["builtin_tv", "builtin_radio"].contains(&owner) {
            let actual = registry
                .query_row("SELECT source FROM refs WHERE id=?", [id], |r| {
                    r.get::<_, String>(0)
                })
                .optional()?;
            if actual.as_deref() != Some(owner) {
                return Err(invalid());
            }
        }
    }
    if let Some(rows) = p["metadata"].as_array() {
        for row in rows {
            if let Some(id) = row["key"].as_str().and_then(|s| s.strip_prefix("i:")) {
                if row["source"] != p["owners"][id] {
                    return Err(invalid());
                }
            }
        }
    }
    for s in strings(&p["series"], 5000)? {
        if !s.split_once(':').is_some_and(|(id, key)| {
            sources.contains_key(id)
                && key.len() == 64
                && key.bytes().all(|b| b.is_ascii_hexdigit())
        }) {
            return Err(invalid());
        }
    }
    let watches = m["watch"].as_array().ok_or_else(invalid)?;
    if watches.len() > 5000 {
        return Err(invalid());
    }
    let mut seen = BTreeSet::new();
    for w in watches {
        let src = w["source"].as_str().ok_or_else(invalid)?;
        let id = w["id"].as_str().ok_or_else(invalid)?;
        let mode = w["mode"].as_str().ok_or_else(invalid)?;
        let event = w["event"].as_str().ok_or_else(invalid)?;
        if !sources.contains_key(src)
            || id.len() != 24
            || !id.bytes().all(|b| b.is_ascii_hexdigit())
            || !matches!(mode, "VOD" | "ARCHIVE")
            || event.encode_utf16().count() > 512
            || mode == "VOD" && !event.is_empty()
            || !seen.insert((src, id, mode, event))
            || ["position", "duration"]
                .iter()
                .any(|k| w[k].as_u64().is_none_or(|n| n > 31_536_000_000))
            || w["updated"].as_u64().is_none()
            || w["completed"].as_bool().is_none()
            || w["hidden"].as_bool().is_none()
        {
            return Err(invalid());
        }
    }
    let p = &m["preferences"];
    if p["data_saver"].as_bool().is_none()
        || p["zap_favorites_only"].as_bool().is_none()
        || !matches!(
            p["subtitle_mode"].as_str(),
            Some("SYSTEM" | "OFF" | "LANGUAGE")
        )
        || ["audio_language", "subtitle_language"].iter().any(|k| {
            p[k].as_str()
                .is_none_or(|s| !crate::preferences::valid_language(s))
        })
    {
        return Err(invalid());
    }
    Ok(())
}

/// Crash recovery is driven by a commit marker in the same SQLite transaction as watch/choices.
pub(super) fn recover(directory: &Path) -> Result<()> {
    let path = directory.join(".restore-journal.json");
    if !path.exists() {
        return Ok(());
    }
    let bytes = fs::read(&path)?;
    if bytes.len() > 32 * 1024 * 1024 {
        bail!("Registro del ripristino non valido.");
    }
    let journal: Value = serde_json::from_slice(&bytes)?;
    let id = journal["id"].as_str().ok_or_else(invalid)?;
    let committed = open(&directory.join("watch.sqlite"))
        .ok()
        .and_then(|db| {
            db.query_row(
                "SELECT EXISTS(SELECT 1 FROM backup_commits WHERE id=?)",
                [id],
                |r| r.get::<_, bool>(0),
            )
            .ok()
        })
        .unwrap_or(false);
    if !committed {
        let sources: BTreeMap<String, Source> = serde_json::from_value(journal["sources"].clone())?;
        let temporary = directory.join(".sources-recover.tmp");
        create(&temporary)?.write_all(&serde_json::to_vec(&sources)?)?;
        fs::rename(temporary, directory.join("sources.json"))?;
        let root = directory.parent().ok_or_else(invalid)?;
        let temporary = root.join(".preferences-recover.tmp");
        create(&temporary)?.write_all(&serde_json::to_vec(&journal["preferences"])?)?;
        fs::rename(temporary, root.join("preferences.json"))?;
        for id in strings(&journal["added"], 10)? {
            if !Library::valid_source(&id) || sources.contains_key(&id) {
                return Err(invalid());
            }
            let _ = fs::remove_file(directory.join(format!("{id}.sqlite")));
        }
    }
    fs::remove_file(path)?;
    Ok(())
}
fn restore(
    j: &Job,
    selection: &Value,
    library: &Library,
    prefs: &PreferenceStore,
    builtins: &[CatalogChannel],
) -> Result<Value> {
    let categories = strings(&selection["categories"], 4)?;
    if categories.is_empty()
        || categories
            .iter()
            .any(|c| !["PLAYLISTS", "PERSONAL", "WATCH", "PREFERENCES"].contains(&c.as_str()))
    {
        bail!("Scegli almeno una categoria da ripristinare.");
    }
    let chosen = if selection["sources"].is_null() {
        None
    } else {
        Some(strings(&selection["sources"], 10)?)
    };
    let manifest: Value = serde_json::from_slice(&fs::read(j.directory.join("manifest.json"))?)?;
    let candidates: BTreeMap<String, Source> =
        serde_json::from_slice(&fs::read(j.directory.join("sources.json"))?)?;
    let _guard = library.operation.lock().unwrap();
    let previous_sources = library.sources.lock().unwrap().clone();
    let previous_prefs = prefs.get();
    let mut next = previous_sources.clone();
    let mut added = Vec::new();
    let mut kept = 0;
    if categories.contains("PLAYLISTS") {
        for (s, candidate) in &candidates {
            if chosen.as_ref().is_some_and(|set| !set.contains(s)) {
                continue;
            }
            if next.contains_key(s) {
                kept += 1;
                continue;
            }
            if library.path(s)?.exists() {
                bail!(
                    "Una lista con lo stesso identificativo è già presente. Riapri Lume e riprova."
                );
            }
            next.insert(s.clone(), candidate.clone());
            added.push(s.clone());
        }
    }
    if next.len() > 10 || next.values().map(|s| s.report.count).sum::<usize>() > MAX_TOTAL {
        bail!("Il ripristino supera il limite di 10 liste o un milione di voci.");
    }
    let mut disk = 0u64;
    for id in next.keys() {
        let path = if added.contains(id) {
            j.directory.join(format!("{id}.sqlite"))
        } else {
            library.path(id)?
        };
        disk = disk
            .checked_add(fs::metadata(path)?.len())
            .ok_or_else(invalid)?;
    }
    if disk > MAX_DISK {
        bail!("Il ripristino supera il limite di 2 GiB delle liste.");
    }
    let registry = open(&j.directory.join("registry.sqlite"))?;
    registry.execute_batch("DROP TABLE IF EXISTS current_refs;CREATE TABLE current_refs(id TEXT,source TEXT,desktop TEXT,content TEXT,kind TEXT,PRIMARY KEY(id,source));BEGIN IMMEDIATE;")?;
    for s in next.values() {
        let path = if added.contains(&s.id) {
            j.directory.join(format!("{}.sqlite", s.id))
        } else {
            library.path(&s.id)?
        };
        let db = open(&path)?;
        let mut rows = db.prepare("SELECT id,content,kind FROM records")?;
        let mut rows = rows.query([])?;
        while let Some(row) = rows.next()? {
            j.check()?;
            let id: String = row.get(0)?;
            registry.execute(
                "INSERT INTO current_refs VALUES(?,?,?,?,?)",
                params![
                    wire::channel_id(&id),
                    s.id,
                    id,
                    if s.settings.content == "auto" {
                        row.get::<_, String>(1)?
                    } else {
                        s.settings.content.clone()
                    },
                    row.get::<_, String>(2)?
                ],
            )?;
        }
    }
    registry.execute_batch("COMMIT")?;
    let builtin_map = builtins
        .iter()
        .map(|c| (wire::builtin_id(c), c.id.clone()))
        .collect::<BTreeMap<_, _>>();
    let resolve = |id: &str, source: &str| -> Result<Option<String>> {
        if ["builtin_tv", "builtin_radio"].contains(&source) {
            return Ok(builtin_map.get(id).cloned());
        }
        Ok(registry
            .query_row(
                "SELECT desktop FROM current_refs WHERE id=? AND source=?",
                params![id, source],
                |r| r.get(0),
            )
            .optional()?)
    };
    let personal = &manifest["personal"];
    let owner = |id: &str| personal["owners"][id].as_str().unwrap_or("");
    let mut next_prefs = previous_prefs.clone();
    let mut restored_favorites = 0;
    let mut skipped = 0;
    if categories.contains("PERSONAL") {
        for id in strings(&personal["favorites"], 200_000)? {
            if let Some(id) = resolve(&id, owner(&id))? {
                if !next_prefs.favorites.contains(&id) {
                    next_prefs.favorites.push(id);
                    restored_favorites += 1;
                }
            } else {
                skipped += 1;
            }
        }
        let mut layout = next_prefs.channel_layout.clone();
        if !layout.is_object() {
            layout = json!({"version":1,"order":[],"hidden":[],"owners":{}});
        }
        let mut order = strings(&layout["order"], 200_000).unwrap_or_default();
        let mut merged_order = layout["order"].as_array().cloned().unwrap_or_default();
        for id in personal["layout"]["order"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
        {
            if let Some(local) = resolve(&id, owner(&id))? {
                if order.insert(local.clone()) {
                    merged_order.push(json!(local));
                }
            } else {
                skipped += 1;
            }
        }
        let mut hidden = layout["hidden"].as_array().cloned().unwrap_or_default();
        let mut hidden_ids = hidden
            .iter()
            .filter_map(|v| v["id"].as_str())
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        for row in personal["layout"]["hidden"].as_array().unwrap() {
            let id = row["id"].as_str().ok_or_else(invalid)?;
            if let Some(local) = resolve(id, owner(id))? {
                if hidden_ids.insert(local.clone()) {
                    let mut row = row.clone();
                    row["id"] = json!(local);
                    hidden.push(row);
                }
            } else {
                skipped += 1;
            }
        }
        next_prefs.channel_layout =
            json!({"version":1,"order":merged_order,"hidden":hidden,"owners":{}});
    }
    if categories.contains("PREFERENCES") {
        let p = &manifest["preferences"];
        next_prefs.audio_language = p["audio_language"].as_str().unwrap().into();
        next_prefs.subtitle_mode = p["subtitle_mode"].as_str().unwrap().to_lowercase();
        next_prefs.subtitle_language = p["subtitle_language"].as_str().unwrap().into();
        next_prefs.zap_favorites_only = p["zap_favorites_only"].as_bool().unwrap();
        if let Ok(desktop) =
            serde_json::from_value::<Preferences>(manifest["desktop"]["preferences"].clone())
        {
            next_prefs.theme = desktop.theme;
            next_prefs.volume = desktop.volume;
            next_prefs.muted = desktop.muted;
            next_prefs.quality = desktop.quality;
        }
    }
    let db = library.watch_db()?;
    drop(library.metadata_db()?);
    db.execute(
        "ATTACH DATABASE ? AS online",
        [library
            .directory
            .join("video-metadata.sqlite")
            .to_string_lossy()
            .as_ref()],
    )?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS backup_commits(id TEXT PRIMARY KEY); BEGIN IMMEDIATE;",
    )?;
    let journal = library.directory.join(".restore-journal.json");
    let result = (|| -> Result<Value> {
        let mut restored_watch = 0;
        if categories.contains("WATCH") {
            for w in manifest["watch"].as_array().unwrap() {
                j.check()?;
                let source = w["source"].as_str().unwrap();
                let id = w["id"].as_str().unwrap();
                let Some(local) = resolve(id, source)? else {
                    skipped += 1;
                    continue;
                };
                let content: Option<String> = registry
                    .query_row(
                        "SELECT content FROM current_refs WHERE id=? AND source=?",
                        params![id, source],
                        |r| r.get(0),
                    )
                    .optional()?;
                let mode = w["mode"].as_str().unwrap().to_lowercase();
                if content.as_deref() != Some(if mode == "vod" { "vod" } else { "live" }) {
                    skipped += 1;
                    continue;
                }
                let count: usize = db.query_row("SELECT COUNT(*) FROM watch", [], |r| r.get(0))?;
                if count >= 5000 {
                    skipped += 1;
                    continue;
                }
                let inserted=db.execute("INSERT OR IGNORE INTO watch(source,channel,mode,event,position,duration,completed,updated,hidden,revision) VALUES(?,?,?,?,?,?,?,?,?,0)",params![source,local,mode,w["event"].as_str().unwrap(),w["position"].as_u64().unwrap()as f64/1000.0,w["duration"].as_u64().unwrap()as f64/1000.0,w["completed"].as_bool().unwrap(),w["updated"].as_u64().unwrap().min(crate::report::now_ms()),w["hidden"].as_bool().unwrap()])?;
                restored_watch += inserted;
                skipped += usize::from(inserted == 0);
            }
        }
        if categories.contains("PERSONAL") {
            for value in strings(&personal["series"], 5000)? {
                let (source, key) = value.split_once(':').unwrap();
                if next.contains_key(source) {
                    db.execute(
                        "INSERT OR IGNORE INTO video_saved VALUES(?,?)",
                        params![source, format!("s:{key}")],
                    )?;
                } else {
                    skipped += 1;
                }
            }
            if let Some(rows) = personal["metadata"].as_array() {
                for r in rows {
                    let source = r["source"].as_str().unwrap();
                    if !next.contains_key(source) {
                        skipped += 1;
                        continue;
                    }
                    let key = r["key"].as_str().unwrap();
                    let key = if let Some(id) = key.strip_prefix("i:") {
                        let Some(id) = resolve(id, source)? else {
                            skipped += 1;
                            continue;
                        };
                        format!("i:{id}")
                    } else {
                        key.into()
                    };
                    db.execute(
                        "INSERT OR IGNORE INTO online.matches VALUES(?,?,?,?,?,'[]',1,?)",
                        params![
                            source,
                            key,
                            r["fingerprint"].as_str().unwrap(),
                            r["id"].as_u64(),
                            r["excluded"].as_bool().unwrap(),
                            u64::MAX / 2
                        ],
                    )?;
                }
            }
            let count: usize = db.query_row(
                "SELECT COUNT(*) FROM online.matches WHERE manual=1",
                [],
                |r| r.get(0),
            )?;
            let saved: usize =
                db.query_row("SELECT COUNT(*) FROM video_saved", [], |r| r.get(0))?;
            if count > 5000 || saved > 5000 {
                bail!("Il ripristino supera il limite di 5.000 elementi salvati.");
            }
        }
        j.check()?;
        let staged_journal = journal.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let publication = (|| -> Result<()> {
            let mut file = create(&staged_journal)?;
            file.write_all(&serde_json::to_vec(&json!({"id":j.id,"sources":previous_sources,"preferences":previous_prefs,"added":added}))?)?;
            file.sync_all()?;
            fs::rename(&staged_journal, &journal)?;
            Ok(())
        })();
        if publication.is_err() {
            let _ = fs::remove_file(&staged_journal);
        }
        publication?;
        for id in &added {
            fs::rename(j.directory.join(format!("{id}.sqlite")), library.path(id)?)?;
        }
        library.save(&next)?;
        *library.sources.lock().unwrap() = next.clone();
        prefs.replace_backup(next_prefs)?;
        library.set_layout(&prefs.get().channel_layout);
        db.execute("INSERT INTO backup_commits VALUES(?)", [&j.id])?;
        db.execute_batch("COMMIT")?;
        let _ = fs::remove_file(&journal);
        library.video_epoch.fetch_add(1, Ordering::AcqRel);
        Ok(
            json!({"playlists":added.len(),"kept":kept,"favorites":restored_favorites,"watch":restored_watch,"skipped":skipped,"preferences":prefs.get()}),
        )
    })();
    if result.is_err() {
        let _ = db.execute_batch("ROLLBACK");
        if journal.exists() {
            recover(&library.directory)?;
            *library.sources.lock().unwrap() = previous_sources;
            library.set_layout(&previous_prefs.channel_layout);
            let _ = prefs.replace_backup(previous_prefs);
            j.phase("error");
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use std::{
        io::Cursor,
        time::{Duration, Instant},
    };
    fn wait(backups: &Backups, id: &str, phase: &str) -> Value {
        let start = Instant::now();
        loop {
            let v = backups.status(id).unwrap();
            if v["phase"] == phase {
                return v;
            }
            assert!(
                !["error", "cancelled"].contains(&v["phase"].as_str().unwrap()),
                "{v}"
            );
            assert!(start.elapsed() < Duration::from_secs(30), "{v}");
            thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    fn independent_android_archive_restores_layout_choices_and_cross_device_reference() {
        let f = super::super::tests::Fixture::new();
        let backups = Backups::new(&f.root).unwrap();
        let prefs = PreferenceStore::open(&f.root).unwrap();
        let input = backups.input().unwrap();
        let id = input["id"].as_str().unwrap();
        backups
            .upload(
                id,
                &mut Cursor::new(
                    base64::engine::general_purpose::STANDARD
                        .decode(
                            include_str!(
                                "../../tests/fixtures/android-backup/android-v1.lumebackup.base64"
                            )
                            .trim(),
                        )
                        .unwrap(),
                ),
            )
            .unwrap();
        backups.prepare(id, "password Android".into()).unwrap();
        wait(&backups, id, "ready");
        backups
            .restore(
                id,
                &json!({"categories":["PLAYLISTS","PERSONAL","WATCH","PREFERENCES"]}),
                &f.library,
                &prefs,
                &[],
            )
            .unwrap();
        let source = "user_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let movie = "personal_000000000000000000000001";
        let live = "personal_000000000000000000000004";
        let hidden = "personal_000000000000000000000003";
        assert_eq!(prefs.get().audio_language, "fr");
        assert_eq!(prefs.get().subtitle_mode, "language");
        assert!(f.library.channel(hidden).is_none());
        assert_eq!(f.library.home(source, &[]).unwrap()["featured"]["id"], live);
        assert_eq!(
            f.library.video_detail(movie, &[]).unwrap()["item"]["position"],
            240.5
        );
        let expected: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/android-backup/watch-reference.json"
        ))
        .unwrap();
        assert_eq!(
            f.library.companion_reference(movie).unwrap()["reference"],
            expected
        );
        let transfer = f.root.join("synthetic-transfer.gz");
        f.library
            .companion_transfer(source, &transfer, &|| Ok(()))
            .unwrap();
        let mut text = String::new();
        flate2::read::GzDecoder::new(File::open(&transfer).unwrap())
            .read_to_string(&mut text)
            .unwrap();
        let mut rows = text.lines();
        let header: Value = serde_json::from_str(rows.next().unwrap()).unwrap();
        assert_eq!(header["format"], "lumetv-playlist");
        assert_eq!(header["metadata"]["count"], 4);
        let rows = rows
            .map(|s| serde_json::from_str::<Value>(s).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0]["headers"]["User-Agent"], "Fixture-UA");
        assert!(!text.contains("favorites"));
        assert!(!text.contains("watch"));

        let metadata = f.library.metadata_db().unwrap();
        assert_eq!(
            metadata
                .query_row(
                    "SELECT selected FROM matches WHERE source=? AND key=?",
                    params![source, format!("i:{movie}")],
                    |r| r.get::<_, u32>(0)
                )
                .unwrap(),
            194
        );
        // The reverse export keeps Android channel IDs, order and visibility.
        let j = backups.new_job("exporting").unwrap();
        export(&j, &f.library, &prefs.get(), &[], "password Android").unwrap();
        let mut plain = Vec::new();
        decrypt(
            &mut backups.download(&j.id).unwrap(),
            &mut plain,
            "password Android",
            &|| Ok(()),
        )
        .unwrap();
        let mut zip = zip::ZipArchive::new(Cursor::new(plain)).unwrap();
        let mut file = zip.by_name("manifest.json").unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        let root: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            root["personal"]["layout"]["order"],
            json!(["000000000000000000000004", "000000000000000000000003"])
        );
        assert_eq!(
            root["personal"]["layout"]["hidden"][0]["id"],
            "000000000000000000000003"
        );
        assert_eq!(root["personal"]["metadata"][0]["id"], 194);
    }
    #[test]
    fn interrupted_restore_rolls_back_publication_before_loading_sources() {
        let f = super::super::tests::Fixture::new();
        let prefs = Preferences::default();
        let added = "user_99999999999999999999999999999999";
        let db = f.library.directory.join(format!("{added}.sqlite"));
        fs::write(&db, b"staged fixture").unwrap();
        let journal =
            json!({"id":"fixture-restore","sources":{},"preferences":prefs,"added":[added]});
        create(&f.library.directory.join(".restore-journal.json"))
            .unwrap()
            .write_all(&serde_json::to_vec(&journal).unwrap())
            .unwrap();
        fs::write(f.root.join("preferences.json"), br#"{"theme":"light"}"#).unwrap();
        drop(f.library.clone());
        let recovered = Library::new(&f.root).unwrap();
        assert!(!db.exists());
        assert!(recovered.snapshot()["sources"]
            .as_array()
            .unwrap()
            .is_empty());
        let restored: Preferences =
            serde_json::from_slice(&fs::read(f.root.join("preferences.json")).unwrap()).unwrap();
        assert_eq!(restored.theme, "dark");
    }
    #[test]
    fn expanded_archive_budget_is_checked_on_actual_reads() {
        let mut total = MAX - 1;
        let mut reader = Expanded {
            input: Cursor::new([1, 2, 3]),
            total: &mut total,
        };
        assert!(reader.read(&mut [0; 8]).is_err());
        assert_eq!(total, MAX + 1);
    }
    #[test]
    fn encrypted_frames_authenticate_order_truncation_password_and_trailing_bytes() {
        let payload = vec![b'Q'; CHUNK + 75];
        let mut encrypted = Vec::new();
        encrypt(
            &mut Cursor::new(&payload),
            &mut encrypted,
            "pàssword lungo",
            &|| Ok(()),
        )
        .unwrap();
        let mut plain = Vec::new();
        decrypt(
            &mut Cursor::new(&encrypted),
            &mut plain,
            "pàssword lungo",
            &|| Ok(()),
        )
        .unwrap();
        assert_eq!(plain, payload);
        for wrong in ["altra password", "pàssword LungO"] {
            assert!(decrypt(
                &mut Cursor::new(&encrypted),
                &mut Vec::new(),
                wrong,
                &|| Ok(())
            )
            .is_err());
        }
        for mutated in [
            encrypted[..encrypted.len() - 1].to_vec(),
            [encrypted.clone(), vec![0]].concat(),
            {
                let mut v = encrypted.clone();
                v[40] ^= 1;
                v
            },
        ] {
            assert!(decrypt(
                &mut Cursor::new(mutated),
                &mut Vec::new(),
                "pàssword lungo",
                &|| Ok(())
            )
            .is_err());
        }
    }
    #[test]
    fn android_backup_roundtrip_preserves_private_lists_saved_series_and_resume_additively() {
        let f = super::super::tests::Fixture::new();
        let source=f.import("#EXTM3U\n#EXTINF:600 type=\"movie\" group-title=\"Cinema\" plot=\"Trama privata\",Film\nhttps://media.example/movie/u/fixture/42.mp4\n#EXTINF:1200 series-title=\"Serie\" season=\"1\" episode=\"1\",Serie S01E01\nhttps://media.example/series/u/fixture/43.mp4\n",None);
        let rows = f.rows(&source);
        let id = rows["rows"][0]["id"].as_str().unwrap();
        let episode = rows["rows"][1]["id"].as_str().unwrap();
        f.library.watch(id, "vod", "", 240.5, 600.0, false).unwrap();
        f.library.video_action(episode, "saved", true).unwrap();
        let prefs = PreferenceStore::open(&f.root).unwrap();
        let mut p = prefs.get();
        p.favorites.push(id.into());
        p.audio_language = "en".into();
        prefs.replace_backup(p).unwrap();
        let backups = Backups::new(&f.root).unwrap();
        let status = backups
            .export(
                f.library.clone(),
                prefs.get(),
                vec![],
                "password prova".into(),
            )
            .unwrap();
        let export_id = status["id"].as_str().unwrap();
        wait(&backups, export_id, "exported");
        let input = backups.input().unwrap();
        let input_id = input["id"].as_str().unwrap();
        backups
            .upload(input_id, &mut backups.download(export_id).unwrap())
            .unwrap();
        backups.prepare(input_id, "password prova".into()).unwrap();
        let preview = wait(&backups, input_id, "ready");
        assert_eq!(preview["favorites"], 1);
        assert_eq!(preview["series"], 1);
        assert!(!preview.to_string().contains("https://"));
        let destination = super::super::tests::Fixture::new();
        let prefs = PreferenceStore::open(&destination.root).unwrap();
        let selection = json!({"categories":["PLAYLISTS","PERSONAL","WATCH","PREFERENCES"]});
        let restored = backups
            .restore(input_id, &selection, &destination.library, &prefs, &[])
            .unwrap();
        assert_eq!(restored["playlists"], 1);
        assert_eq!(restored["watch"], 1);
        assert!(prefs.get().favorites.contains(&id.to_string()));
        assert_eq!(prefs.get().audio_language, "en");
        let detail = destination
            .library
            .video_detail(id, &prefs.get().favorites)
            .unwrap();
        assert_eq!(detail["item"]["position"], 240.5);
        assert_eq!(detail["item"]["description"], "Trama privata");
        assert_eq!(
            destination.library.video_detail(episode, &[]).unwrap()["item"]["saved"],
            true
        );
        // A second preview of the same document must preserve the destination's existing watch value.
        destination
            .library
            .watch(id, "vod", "", 300.0, 600.0, false)
            .unwrap();
        let input = backups.input().unwrap();
        let again = input["id"].as_str().unwrap();
        backups
            .upload(again, &mut backups.download(export_id).unwrap())
            .unwrap();
        backups.prepare(again, "password prova".into()).unwrap();
        wait(&backups, again, "ready");
        let restored = backups
            .restore(again, &selection, &destination.library, &prefs, &[])
            .unwrap();
        assert_eq!(restored["playlists"], 0);
        assert_eq!(restored["kept"], 1);
        assert_eq!(
            destination.library.video_detail(id, &[]).unwrap()["item"]["position"],
            300.0
        );
    }
    #[test]
    fn traversal_entries_are_rejected_before_any_library_change() {
        use zip::write::SimpleFileOptions;
        let f = super::super::tests::Fixture::new();
        let backups = Backups::new(&f.root).unwrap();
        let j = backups.new_job("uploaded").unwrap();
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file("../sources.json", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"{}").unwrap();
        let archive = zip.finish().unwrap().into_inner();
        let mut encrypted = create(&j.directory.join("input.lumebackup")).unwrap();
        encrypt(
            &mut Cursor::new(archive),
            &mut encrypted,
            "password prova",
            &|| Ok(()),
        )
        .unwrap();
        drop(encrypted);
        assert!(prepare(&j, "password prova").is_err());
        assert_eq!(f.library.snapshot()["sources"].as_array().unwrap().len(), 0);
    }
}
