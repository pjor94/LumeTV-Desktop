// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use anyhow::{anyhow, bail, Context as _, Result};
use reqwest::Method;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Condvar, Mutex,
    },
    thread,
};

const DISPLAY_COLUMNS: &str = r#"
    id,name,number,kind,content,position,json_extract(payload,'$.groups'),
    COALESCE(json_extract(payload,'$.metadata.tvg_id'),''),
    COALESCE(json_extract(payload,'$.metadata.tvg_name'),''),
    (trim(COALESCE(json_extract(payload,'$.logo'),''))<>'' OR
     trim(COALESCE(json_extract(payload,'$.metadata.attributes."tvg-logo"'),''))<>''),
    (COALESCE(json_array_length(json_extract(payload,'$.metadata.guide_urls')),0)>0 OR
     trim(COALESCE(json_extract(payload,'$.metadata.attributes."url-tvg"'),''))<>'' OR
     trim(COALESCE(json_extract(payload,'$.metadata.attributes."x-tvg-url"'),''))<>'')
"#;

#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ImportConfig {
    pub source_id: String,
    pub revision: u64,
    pub name: String,
    pub url: Option<String>,
    pub settings: Settings,
}
#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CatalogQuery {
    pub source: String,
    pub kind: String,
    pub content: String,
    pub query: String,
    pub group: String,
    pub offset: usize,
    pub limit: usize,
    pub favorites: bool,
}
#[derive(Clone, Serialize)]
pub struct JobStatus {
    pub id: String,
    pub name: String,
    pub source_id: String,
    pub phase: String,
    pub bytes: u64,
    pub entries: usize,
    pub report: Option<ImportReport>,
    pub error: String,
}
struct Job {
    network: Mutex<Option<Arc<network::Context>>>,
    interrupt: Mutex<Option<rusqlite::InterruptHandle>>,
    config: ImportConfig,
    status: Mutex<JobStatus>,
    cancel: AtomicBool,
    draft: PathBuf,
    input: PathBuf,
    auto: bool,
    etag: Mutex<(String, String)>,
}
impl Job {
    fn check(&self) -> Result<()> {
        if self.cancel.load(Ordering::Acquire) {
            bail!("Importazione annullata.");
        }
        Ok(())
    }
    fn phase(&self, phase: &str) {
        self.status.lock().expect("import status").phase = phase.into();
    }
    fn cleanup(&self) {
        for path in [&self.draft, &self.input] {
            let _ = fs::remove_file(path);
            let _ = fs::remove_file(format!("{}-journal", path.display()));
        }
    }
}
pub struct Library {
    pub(super) directory: PathBuf,
    pub(super) sources: Mutex<BTreeMap<String, Source>>,
    pub(super) operation: Mutex<()>,
    contexts: Mutex<BTreeMap<String, Arc<network::Context>>>,
    jobs: Mutex<BTreeMap<String, Arc<Job>>>,
    pub(super) guide_loading: Mutex<std::collections::HashSet<String>>,
    pub(super) public_guide: std::sync::RwLock<std::sync::Weak<crate::guide::GuideService>>,
    logo_downloads: Mutex<usize>,
    logo_ready: Condvar,
    pub(super) video_online: Mutex<()>,
    layout: Mutex<Value>,
    pub(super) video_epoch: std::sync::atomic::AtomicU64,
    pub(super) video_retry: std::sync::atomic::AtomicU64,
}
struct LogoSlot<'a>(&'a Library);
impl Drop for LogoSlot<'_> {
    fn drop(&mut self) {
        let mut count = self.0.logo_downloads.lock().expect("logo downloads");
        *count -= 1;
        self.0.logo_ready.notify_one();
    }
}
fn now() -> u64 {
    crate::report::now_ms()
}
fn private_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
pub(super) fn private_file(_path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(_path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
pub(super) fn open(path: &Path) -> Result<Connection> {
    let db = Connection::open(path)?;
    db.busy_timeout(std::time::Duration::from_secs(10))?;
    db.execute_batch(
        "PRAGMA temp_store=FILE; PRAGMA cache_size=-4096; PRAGMA journal_mode=DELETE;",
    )?;
    private_file(path)?;
    Ok(db)
}
impl Library {
    pub fn new(root: &Path) -> Result<Arc<Self>> {
        let directory = root.join("personal");
        private_directory(&directory)?;
        super::backup::recover(&directory)?;
        let sources = if directory.join("sources.json").exists() {
            let bytes = fs::read(directory.join("sources.json"))?;
            if bytes.len() > 16 * 1024 * 1024 {
                bail!("Archivio delle liste personali troppo grande.");
            }
            let sources: BTreeMap<String, Source> = serde_json::from_slice(&bytes)
                .context("Archivio delle liste personali non leggibile")?;
            if sources.len() > 10
                || sources
                    .iter()
                    .any(|(id, s)| id != &s.id || !Self::valid_source(id))
            {
                bail!("Archivio delle sorgenti non valido.");
            }
            sources
        } else {
            BTreeMap::new()
        };
        let library = Arc::new(Self {
            directory,
            sources: Mutex::new(sources),
            operation: Mutex::new(()),
            contexts: Mutex::new(BTreeMap::new()),
            jobs: Mutex::new(BTreeMap::new()),
            guide_loading: Mutex::new(Default::default()),
            public_guide: std::sync::RwLock::new(std::sync::Weak::new()),
            logo_downloads: Mutex::new(0),
            logo_ready: Condvar::new(),
            video_online: Mutex::new(()),
            layout: Mutex::new(serde_json::json!({"order":[],"hidden":[]})),
            video_epoch: std::sync::atomic::AtomicU64::new(0),
            video_retry: std::sync::atomic::AtomicU64::new(0),
        });
        for source in library.sources.lock().expect("sources").values() {
            let target = library.path(&source.id)?;
            let backup = library
                .directory
                .join(format!(".backup-{}.sqlite", source.id));
            if backup.exists() {
                let matches = target.exists()
                    && open(&target)
                        .ok()
                        .and_then(|db| {
                            db.query_row(
                                "SELECT value FROM metadata WHERE key='source_snapshot'",
                                [],
                                |r| r.get::<_, String>(0),
                            )
                            .ok()
                        })
                        .is_some_and(|value| value == source.snapshot);
                if matches {
                    fs::remove_file(backup)?;
                } else {
                    let _ = fs::remove_file(&target);
                    fs::rename(backup, target)?;
                }
            }
        }
        for file in fs::read_dir(&library.directory)? {
            let file = file?;
            let name = file.file_name().to_string_lossy().into_owned();
            if name.starts_with(".draft-")
                || name.starts_with(".input-")
                || name.starts_with(".restore-journal.") && name.ends_with(".tmp")
            {
                let _ = fs::remove_file(file.path());
            }
        }
        Ok(library)
    }
    pub(super) fn valid_source(id: &str) -> bool {
        id.strip_prefix("user_")
            .is_some_and(|v| v.len() == 32 && v.bytes().all(|b| b.is_ascii_hexdigit()))
    }
    pub(super) fn path(&self, id: &str) -> Result<PathBuf> {
        if !Self::valid_source(id) {
            bail!("Sorgente non valida.");
        }
        Ok(self.directory.join(format!("{id}.sqlite")))
    }
    pub(super) fn save(&self, sources: &BTreeMap<String, Source>) -> Result<()> {
        let temp = self
            .directory
            .join(format!(".sources-{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut file = fs::File::create(&temp)?;
            private_file(&temp)?;
            file.write_all(&serde_json::to_vec(sources)?)?;
            file.sync_all()?;
            fs::rename(&temp, self.directory.join("sources.json"))?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }
    pub(crate) fn set_layout(&self, layout: &Value) {
        *self.layout.lock().unwrap() = layout.clone();
    }
    pub(crate) fn is_hidden(&self, id: &str) -> bool {
        self.layout.lock().unwrap()["hidden"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|r| r["id"] == id))
    }
    pub fn snapshot(&self) -> Value {
        let sources = self.sources.lock().expect("personal sources");
        let imports = self
            .jobs
            .lock()
            .expect("jobs")
            .values()
            .filter(|job| !job.auto)
            .filter_map(|job| {
                let mut status = job.status.lock().expect("status").clone();
                if ["done", "cancelled", "error"].contains(&status.phase.as_str()) {
                    return None;
                }
                if let Some(report) = status.report.as_mut() {
                    report.guide_urls.clear();
                }
                Some(status)
            })
            .collect::<Vec<_>>();
        json!({"sources":sources.values().map(|s|{let mut value=s.public();value["guide"]=self.presented_guide(s);value}).collect::<Vec<_>>(),"imports":imports,"metadata_revision":self.public_metadata_revision(),"layout_revision":format!("{:x}",Sha256::digest(self.layout.lock().unwrap().to_string())),"max_sources":10,"max_entries":MAX_ENTRIES,"max_total":MAX_TOTAL,"max_bytes":MAX_BYTES})
    }
    pub fn detail(&self, id: &str) -> Result<Value> {
        let source = self.source(id)?;
        Ok(
            json!({"id":source.id,"name":source.name,"enabled":source.enabled,"revision":source.revision,"url":source.url,"settings":source.settings}),
        )
    }
    pub(super) fn source(&self, id: &str) -> Result<Source> {
        self.sources
            .lock()
            .expect("personal sources")
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow!("La lista non è più disponibile."))
    }
    pub(super) fn context(&self, source: &Source) -> Result<Arc<network::Context>> {
        let _operation = self.operation.lock().expect("operation");
        let current = self.source(&source.id)?;
        if !current.enabled || current.revision != source.revision {
            bail!("La lista è disattivata.");
        }
        let mut contexts = self.contexts.lock().expect("personal contexts");
        if let Some(context) = contexts.get(&source.id) {
            if serde_json::to_value(&context.settings)? == serde_json::to_value(&source.settings)? {
                return Ok(context.clone());
            }
        }
        let mut context = network::Context::new(&source.settings)?;
        if let Some(old) = contexts.get(&source.id) {
            Arc::get_mut(&mut context)
                .expect("new context")
                .inherit_media(old);
        }
        contexts.insert(source.id.clone(), context.clone());
        Ok(context)
    }

    #[cfg(test)]
    pub(super) fn fixture_context(&self, id: &str, context: Arc<network::Context>) {
        self.contexts.lock().unwrap().insert(id.into(), context);
    }
    fn revoke(&self, id: &str) {
        if let Some(context) = self.contexts.lock().expect("personal contexts").remove(id) {
            context.revoke();
        }
    }
    fn cancel_source_imports(&self, id: &str, except: Option<&str>) {
        for (key, job) in self.jobs.lock().expect("jobs").iter() {
            if job.config.source_id != id || except == Some(key.as_str()) {
                continue;
            }
            let mut status = job.status.lock().expect("status");
            if ["done", "error", "cancelled"].contains(&status.phase.as_str()) {
                continue;
            }
            job.cancel.store(true, Ordering::Release);
            if let Some(context) = job.network.lock().expect("network").as_ref() {
                context.revoke();
            }
            if let Some(interrupt) = job.interrupt.lock().expect("interrupt").as_ref() {
                interrupt.interrupt();
            }
            if ["ready", "upload"].contains(&status.phase.as_str()) {
                job.cleanup();
                status.phase = "cancelled".into();
            } else {
                status.phase = "cancelling".into();
            }
        }
    }
    pub fn prepare(self: &Arc<Self>, mut config: ImportConfig, auto: bool) -> Result<Value> {
        config.settings.validate()?;
        config.name = config.name.trim().into();
        if config.name.is_empty()
            || config.name.chars().count() > 160
            || config.name.chars().any(char::is_control)
        {
            bail!("Inserisci un nome della lista, fino a 160 caratteri.");
        }
        if let Some(url) = &config.url {
            network::validate_url(url, config.settings.allow_http)?;
        }
        if !config.source_id.is_empty() {
            let source = self.source(&config.source_id)?;
            if source.revision != config.revision {
                bail!("La lista è cambiata. Riaprila prima di sostituirla.");
            }
        } else if self.sources.lock().expect("sources").len() >= 10 {
            bail!("Puoi aggiungere fino a dieci liste personali.");
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        let mut jobs = self.jobs.lock().expect("import jobs");
        jobs.retain(|_, j| {
            !["done", "error", "cancelled"]
                .contains(&j.status.lock().expect("status").phase.as_str())
        });
        if jobs.len() >= 4 {
            bail!("Completa o annulla un’importazione prima di iniziarne un’altra.");
        }
        let job = Arc::new(Job {
            network: Mutex::new(None),
            interrupt: Mutex::new(None),
            config: config.clone(),
            status: Mutex::new(JobStatus {
                id: id.clone(),
                name: config.name.clone(),
                source_id: config.source_id.clone(),
                phase: if config.url.is_some() {
                    "reading"
                } else {
                    "upload"
                }
                .into(),
                bytes: 0,
                entries: 0,
                report: None,
                error: String::new(),
            }),
            cancel: AtomicBool::new(false),
            draft: self.directory.join(format!(".draft-{id}.sqlite")),
            input: self.directory.join(format!(".input-{id}.m3u")),
            auto,
            etag: Mutex::new(Default::default()),
        });
        jobs.insert(id.clone(), job.clone());
        drop(jobs);
        if config.url.is_some() {
            self.spawn_scan(job);
        }
        self.job(&id)
    }
    pub fn job(&self, id: &str) -> Result<Value> {
        let jobs = self.jobs.lock().expect("jobs");
        let mut status = jobs
            .get(id)
            .ok_or_else(|| anyhow!("Importazione non disponibile."))?
            .status
            .lock()
            .expect("status")
            .clone();
        if let Some(report) = status.report.as_mut() {
            report.guide_urls.clear();
        }
        let value = serde_json::to_value(status)?;
        Ok(value)
    }
    pub fn upload(self: &Arc<Self>, id: &str, reader: &mut dyn Read) -> Result<Value> {
        let job = self
            .jobs
            .lock()
            .expect("jobs")
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow!("Importazione non disponibile."))?;
        {
            let mut status = job.status.lock().expect("status");
            if status.phase != "upload" {
                bail!("Questa importazione non attende un file.");
            }
            status.phase = "uploading".into();
        }
        let result = (|| -> Result<()> {
            let mut output = fs::File::create(&job.input)?;
            private_file(&job.input)?;
            let mut bytes = 0;
            let mut buffer = [0u8; 65536];
            loop {
                job.check()?;
                let n = reader.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                bytes += n as u64;
                if bytes > MAX_BYTES {
                    bail!("Il documento supera 256 MiB.");
                }
                output.write_all(&buffer[..n])?;
                job.status.lock().expect("status").bytes = bytes;
            }
            output.sync_all()?;
            Ok(())
        })();
        if let Err(error) = result {
            job.cleanup();
            self.fail(&job, &error.to_string());
            return Err(error);
        }
        self.spawn_scan(job);
        self.job(id)
    }
    fn fail(&self, job: &Job, message: &str) {
        let mut status = job.status.lock().expect("status");
        status.phase = if job.cancel.load(Ordering::Acquire) {
            "cancelled"
        } else {
            "error"
        }
        .into();
        status.error = message.into();
    }
    fn spawn_scan(self: &Arc<Self>, job: Arc<Job>) {
        let library = self.clone();
        thread::spawn(move || {
            if let Err(error) = library.scan(&job) {
                job.cleanup();
                library.fail(&job, &error.to_string());
                if job.auto && !job.cancel.load(Ordering::Acquire) {
                    library.source_error(
                        &job.config.source_id,
                        &error.to_string(),
                        now() + 3_600_000,
                    );
                }
            } else if job.auto && job.status.lock().expect("status").phase == "ready" {
                if let Err(error) = library.commit_job(&job) {
                    job.cleanup();
                    library.fail(&job, &error.to_string());
                }
            }
        });
    }
    fn scan(&self, job: &Job) -> Result<()> {
        job.check()?;
        job.phase("reading");
        let mut context = network::Context::new(&job.config.settings)?;
        if job.auto {
            if let Ok(source) = self.source(&job.config.source_id) {
                if let Ok(previous) = self.context(&source) {
                    Arc::get_mut(&mut context)
                        .expect("new import context")
                        .inherit_download(&previous);
                }
            }
        }
        *job.network.lock().expect("import context") = Some(context.clone());
        job.check()?;
        let (base, input): (Option<String>, Box<dyn Read + Send>) =
            if let Some(url) = &job.config.url {
                let previous = self.source(&job.config.source_id).ok();
                let mut headers = Headers::new();
                if job.auto {
                    if let Some(s) = &previous {
                        if !s.etag.is_empty() {
                            headers.insert("If-None-Match".into(), s.etag.clone());
                        }
                        if !s.modified.is_empty() {
                            headers.insert("If-Modified-Since".into(), s.modified.clone());
                        }
                    }
                }
                let response =
                    context.request(url, url, Resource::Playlist, &headers, Method::GET, None)?;
                if response.status().as_u16() == 304 && job.auto {
                    let _guard = self.operation.lock().expect("personal operation");
                    let mut sources = self.sources.lock().expect("sources");
                    if let Some(source) = sources.get_mut(&job.config.source_id) {
                        if source.revision == job.config.revision {
                            source.checked_at = now();
                            source.error.clear();
                            self.save(&sources)?;
                        }
                    }
                    job.phase("done");
                    return Ok(());
                }
                if !response.status().is_success() {
                    let message = match response.status().as_u16() {
                        401 | 403 => "Accesso negato. Controlla l’indirizzo e i dati di accesso.",
                        404 | 410 => "La lista non è più disponibile a questo indirizzo.",
                        429 => "Il servizio richiede una pausa. Riprova più tardi.",
                        _ => "Il server della lista non risponde.",
                    };
                    if response.status().as_u16() == 429 && job.auto {
                        let wait = response
                            .headers()
                            .get("Retry-After")
                            .and_then(|s| s.to_str().ok())
                            .and_then(|s| s.parse::<u64>().ok())
                            .unwrap_or(3600)
                            .clamp(60, 86400);
                        self.source_error(&job.config.source_id, message, now() + wait * 1000);
                    }
                    bail!(message);
                }
                if response.content_length().is_some_and(|n| n > MAX_BYTES) {
                    bail!("La lista supera 256 MiB.");
                }
                *job.etag.lock().expect("etag") = (
                    response
                        .headers()
                        .get("ETag")
                        .and_then(|h| h.to_str().ok())
                        .unwrap_or("")
                        .chars()
                        .take(1024)
                        .collect(),
                    response
                        .headers()
                        .get("Last-Modified")
                        .and_then(|h| h.to_str().ok())
                        .unwrap_or("")
                        .chars()
                        .take(200)
                        .collect(),
                );
                (
                    Some(response.url().to_string()),
                    parser::response_reader(response),
                )
            } else {
                (None, Box::new(fs::File::open(&job.input)?))
            };
        let db = open(&job.draft)?;
        *job.interrupt.lock().expect("import interrupt") = Some(db.get_interrupt_handle());
        db.execute_batch("PRAGMA max_page_count=262144;
            CREATE TABLE records(id TEXT NOT NULL,name TEXT NOT NULL,number INTEGER,kind TEXT NOT NULL,content TEXT NOT NULL,position INTEGER NOT NULL,payload TEXT NOT NULL,identity1 TEXT,identity2 TEXT,identity3 TEXT,seen INTEGER NOT NULL,active INTEGER NOT NULL);
            CREATE TABLE groups(name TEXT NOT NULL,position INTEGER NOT NULL,PRIMARY KEY(name,position));
            CREATE TEMP TABLE fingerprints(value TEXT PRIMARY KEY);
            PRAGMA temp.cache_size=-4096;
            BEGIN IMMEDIATE;")?;
        let imported_at = now();
        let mut insert = db.prepare("INSERT INTO records VALUES(?,?,?,?,?,?,?,?,?,?,?,1)")?;
        let mut group = db.prepare("INSERT OR IGNORE INTO groups VALUES(?,?)")?;
        let mut fingerprint = db.prepare("INSERT OR IGNORE INTO fingerprints VALUES(?)")?;
        let mut sink = |entry: &Entry| -> Result<bool> {
            let kind = if entry.kind == crate::model::MediaKind::Radio {
                "radio"
            } else {
                "tv"
            };
            let key = |value: &str| {
                format!(
                    "{:x}",
                    Sha256::digest(format!("{kind}|{}|{value}", entry.content))
                )
            };
            if fingerprint.execute([key(&format!("{}|{}", entry.url, entry.name))])? == 0 {
                return Ok(false);
            }
            let id1 = (!entry.metadata.tvg_id.is_empty()).then(|| key(&entry.metadata.tvg_id));
            let id2 = key(&entry.url);
            let id3 = key(&format!(
                "{}|{}",
                entry.name.to_lowercase(),
                entry.groups.join("|")
            ));
            insert.execute(params![
                entry.id,
                entry.name,
                entry.number,
                kind,
                entry.content,
                entry.position,
                serde_json::to_string(entry)?,
                id1,
                id2,
                id3,
                imported_at
            ])?;
            for name in &entry.groups {
                group.execute(params![name, entry.position])?;
            }
            Ok(true)
        };
        let check = || job.check();
        let progress = |bytes, entries| {
            let mut status = job.status.lock().expect("status");
            status.bytes = bytes;
            status.entries = entries;
        };
        let mut scanner =
            parser::Scanner::new(&job.config.settings, context, &mut sink, &check, &progress);
        scanner.scan(input, base.as_deref())?;
        let report = scanner.report;
        if report.count == 0 {
            bail!("Nessuna voce importabile. Controlla formato, accesso e consensi HTTP.");
        }
        job.phase("indexing");
        job.check()?;
        db.execute_batch("CREATE UNIQUE INDEX by_id ON records(id); CREATE INDEX by_position ON records(active,kind,content,position);
            CREATE INDEX match1 ON records(identity1); CREATE INDEX match2 ON records(identity2); CREATE INDEX match3 ON records(identity3); COMMIT;")?;
        drop(fingerprint);
        drop(group);
        drop(insert);
        drop(db);
        job.check()?;
        let mut status = job.status.lock().expect("status");
        status.bytes = report.bytes;
        status.entries = report.scanned;
        status.report = Some(report);
        status.phase = "ready".into();
        Ok(())
    }
    pub fn commit(self: &Arc<Self>, id: &str) -> Result<Value> {
        let job = self
            .jobs
            .lock()
            .expect("jobs")
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow!("Importazione non disponibile."))?;
        {
            let mut status = job.status.lock().expect("status");
            if status.phase != "ready" {
                bail!("Attendi la verifica della lista.");
            }
            status.phase = "preparing".into();
        }
        let library = self.clone();
        let copy = job.clone();
        thread::spawn(move || {
            if let Err(error) = library.commit_job(&copy) {
                copy.cleanup();
                library.fail(&copy, &error.to_string());
            }
        });
        self.job(id)
    }
    fn commit_job(&self, job: &Job) -> Result<()> {
        job.check()?;
        job.phase("preparing");
        let _guard = self.operation.lock().expect("personal operation");
        let mut sources = self.sources.lock().expect("sources");
        let old = sources.get(&job.config.source_id).cloned();
        if !job.config.source_id.is_empty()
            && old
                .as_ref()
                .is_none_or(|s| s.revision != job.config.revision)
        {
            bail!("La lista è cambiata durante l’importazione. Verificala di nuovo.");
        }
        if old.is_none() && sources.len() >= 10 {
            bail!("Sono già presenti dieci liste.");
        }
        let report = job
            .status
            .lock()
            .expect("status")
            .report
            .clone()
            .ok_or_else(|| anyhow!("Anteprima non disponibile."))?;
        if sources
            .values()
            .filter(|s| s.id != job.config.source_id)
            .map(|s| s.report.count)
            .sum::<usize>()
            + report.count
            > MAX_TOTAL
        {
            bail!("Le liste superano un milione di voci complessive.");
        }
        let used = fs::read_dir(&self.directory)?
            .filter_map(|f| f.ok())
            .filter(|f| f.file_name().to_string_lossy().starts_with("user_"))
            .filter_map(|f| f.metadata().ok())
            .map(|m| m.len())
            .sum::<u64>();
        if fs::metadata(&job.draft)?.len() > MAX_DATABASE
            || used + fs::metadata(&job.draft)?.len() > MAX_DISK
        {
            bail!("Spazio degli archivi personali esaurito.");
        }
        let id = old
            .as_ref()
            .map(|s| s.id.clone())
            .unwrap_or_else(|| format!("user_{}", uuid::Uuid::new_v4().simple()));
        let snapshot = uuid::Uuid::new_v4().simple().to_string();
        let target = self.path(&id)?;
        {
            let db = open(&job.draft)?;
            *job.interrupt.lock().expect("interrupt") = Some(db.get_interrupt_handle());
            db.execute_batch(
                "CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT)",
            )?;
            db.execute(
                "INSERT OR REPLACE INTO metadata VALUES('source_snapshot',?)",
                [&snapshot],
            )?;
            if target.exists() {
                db.execute(
                    "ATTACH DATABASE ? AS prior",
                    [target.to_string_lossy().as_ref()],
                )?;
                db.execute_batch("BEGIN IMMEDIATE; CREATE TEMP TABLE matches(row_id INTEGER PRIMARY KEY,id TEXT UNIQUE);")?;
                let fresh: usize =
                    db.query_row("SELECT COUNT(*) FROM records WHERE active=1", [], |row| {
                        row.get(0)
                    })?;
                for column in ["identity1", "identity2", "identity3"] {
                    db.execute_batch(&format!("INSERT OR IGNORE INTO matches SELECT fresh.rid,old.id FROM
                        (SELECT {column} AS k,MIN(rowid) AS rid,COUNT(*) AS n FROM records WHERE {column} IS NOT NULL AND rowid NOT IN(SELECT row_id FROM matches) GROUP BY {column}) fresh
                        JOIN (SELECT {column} AS k,MIN(id) AS id,COUNT(*) AS n FROM prior.records WHERE {column} IS NOT NULL AND id NOT IN(SELECT id FROM matches) GROUP BY {column}) old ON fresh.k=old.k WHERE fresh.n=1 AND old.n=1;"))?;
                    let matched: usize =
                        db.query_row("SELECT COUNT(*) FROM matches", [], |row| row.get(0))?;
                    if matched == fresh {
                        break;
                    }
                }
                db.execute_batch("DROP INDEX by_id; UPDATE records SET id=(SELECT id FROM matches WHERE matches.row_id=records.rowid) WHERE rowid IN(SELECT row_id FROM matches); CREATE UNIQUE INDEX by_id ON records(id);")?;
                db.execute("INSERT OR IGNORE INTO records SELECT id,name,number,kind,content,position,payload,identity1,identity2,identity3,seen,0 FROM prior.records WHERE id NOT IN(SELECT id FROM records) AND seen>=? ORDER BY seen DESC LIMIT 5000",[now().saturating_sub(30*86_400_000)])?;
                db.execute_batch("CREATE TABLE IF NOT EXISTS video_added(id TEXT PRIMARY KEY,added INTEGER NOT NULL);")?;
                let has_added: bool = db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM prior.sqlite_master WHERE name='video_added')",
                    [],
                    |r| r.get(0),
                )?;
                if has_added {
                    db.execute_batch("INSERT OR IGNORE INTO video_added SELECT id,added FROM prior.video_added WHERE id IN(SELECT id FROM records);")?;
                } else {
                    let has_video: bool = db.query_row(
                        "SELECT EXISTS(SELECT 1 FROM prior.sqlite_master WHERE name='video')",
                        [],
                        |r| r.get(0),
                    )?;
                    if has_video {
                        db.execute_batch("INSERT OR IGNORE INTO video_added SELECT id,added FROM prior.video WHERE id IN(SELECT id FROM records);")?;
                    } else {
                        db.execute_batch("INSERT OR IGNORE INTO video_added SELECT id,seen FROM prior.records WHERE active=1 AND id IN(SELECT id FROM records);")?;
                    }
                }
                db.execute_batch("COMMIT; DETACH DATABASE prior;")?;
            }
            db.execute(
                "INSERT OR REPLACE INTO metadata VALUES('content_rules',?)",
                [serde_json::to_string(&(
                    1,
                    &job.config.settings.vod_detection,
                    &job.config.settings.group_content,
                ))?],
            )?;
            super::video::index(&db, &job.config.settings.content, now(), &|| job.check())?;
        }
        job.check()?;
        let mut cached = network::Context::new(&job.config.settings)?;
        job.phase("saving");
        let (etag, modified) = job.etag.lock().expect("etag").clone();
        let source = Source {
            android_descriptor: Value::Null,
            snapshot,
            id: id.clone(),
            name: job.config.name.clone(),
            url: job.config.url.clone(),
            enabled: old.as_ref().is_none_or(|s| s.enabled),
            revision: old.as_ref().map_or(1, |s| s.revision + 1),
            updated_at: now(),
            checked_at: now(),
            retry_at: 0,
            etag,
            modified,
            settings: job.config.settings.clone(),
            report,
            guide: old.as_ref().map(|s| s.guide.clone()).unwrap_or_default(),
            error: String::new(),
        };
        let backup = self.directory.join(format!(".backup-{id}.sqlite"));
        if target.exists() {
            fs::rename(&target, &backup)?;
        }
        if let Err(error) = fs::rename(&job.draft, &target) {
            if backup.exists() {
                let _ = fs::rename(&backup, &target);
            }
            return Err(error.into());
        }
        let prior = sources.insert(id.clone(), source);
        if let Err(error) = self.save(&sources) {
            let _ = fs::remove_file(&target);
            if backup.exists() {
                let _ = fs::rename(&backup, &target);
            }
            if let Some(prior) = prior {
                sources.insert(id, prior);
            } else {
                sources.remove(&id);
            }
            return Err(error);
        }
        if let Some(previous) = &old {
            let mut before = previous.settings.clone();
            let mut after = job.config.settings.clone();
            before.guide = Default::default();
            after.guide = Default::default();
            before.access.remove(&Resource::Guide);
            after.access.remove(&Resource::Guide);
            if serde_json::to_value(before)? != serde_json::to_value(after)? {
                self.revoke(&previous.id);
            }
        }
        {
            let mut contexts = self.contexts.lock().expect("contexts");
            if let Some(previous) = contexts.get(&id) {
                Arc::get_mut(&mut cached)
                    .expect("new context")
                    .inherit_media(previous);
            }
            if let Some(download) = job.network.lock().expect("network").as_ref() {
                Arc::get_mut(&mut cached)
                    .expect("new context")
                    .inherit_download(download);
            }
            contexts.insert(id.clone(), cached);
        }
        let _ = fs::remove_file(backup);
        let _ = fs::remove_file(&job.input);
        let job_id = job.status.lock().expect("status").id.clone();
        self.cancel_source_imports(&id, Some(&job_id));
        let mut status = job.status.lock().expect("status");
        status.source_id = id;
        status.phase = "done".into();
        Ok(())
    }
    pub fn cancel(&self, id: &str) -> Result<Value> {
        let job = self
            .jobs
            .lock()
            .expect("jobs")
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow!("Importazione non disponibile."))?;
        {
            let mut status = job.status.lock().expect("status");
            if status.phase == "saving" {
                bail!("Il salvataggio finale è già in corso.");
            }
            if status.phase == "done" {
                drop(status);
                return self.job(id);
            }
            job.cancel.store(true, Ordering::Release);
            if ["ready", "upload"].contains(&status.phase.as_str()) {
                job.cleanup();
                status.phase = "cancelled".into();
            } else {
                status.phase = "cancelling".into();
            }
        }
        if let Some(context) = job.network.lock().expect("network").as_ref() {
            context.revoke();
        }
        if let Some(interrupt) = job.interrupt.lock().expect("interrupt").as_ref() {
            interrupt.interrupt();
        }
        self.job(id)
    }
    pub fn edit(
        &self,
        id: &str,
        revision: u64,
        name: &str,
        enabled: bool,
        settings: Settings,
    ) -> Result<Value> {
        settings.validate()?;
        if name.trim().is_empty()
            || name.chars().count() > 160
            || name.chars().any(char::is_control)
        {
            bail!("Nome della lista non valido.");
        }
        let _guard = self.operation.lock().expect("operation");
        let mut sources = self.sources.lock().expect("sources");
        let mut next = sources.clone();
        let source = next
            .get_mut(id)
            .ok_or_else(|| anyhow!("Lista non disponibile."))?;
        if source.revision != revision {
            bail!("La lista è stata modificata. Riaprila prima di salvare.");
        }
        let mut previous_access = source.settings.clone();
        let mut next_access = settings.clone();
        previous_access.guide = Default::default();
        next_access.guide = Default::default();
        previous_access.access.remove(&Resource::Guide);
        next_access.access.remove(&Resource::Guide);
        let revoke = !enabled
            || serde_json::to_value(&previous_access)? != serde_json::to_value(&next_access)?;
        let db = open(&self.path(id)?)?;
        super::video::ensure_content(&db, &settings)?;
        source.report.vod = db.query_row(
            "SELECT COUNT(*) FROM records WHERE active=1 AND content='vod'",
            [],
            |r| r.get(0),
        )?;
        source.name = name.trim().into();
        source.enabled = enabled;
        source.settings = settings;
        source.revision += 1;
        source.error.clear();
        let next_settings = source.settings.clone();
        self.save(&next)?;
        *sources = next;
        self.cancel_source_imports(id, None);
        if revoke {
            self.revoke(id);
        } else if let Some(context) = self.contexts.lock().expect("contexts").get(id) {
            if serde_json::to_value(&context.settings.guide)?
                != serde_json::to_value(&next_settings.guide)?
                || context
                    .settings
                    .access
                    .get(&Resource::Guide)
                    .map(serde_json::to_value)
                    .transpose()?
                    != next_settings
                        .access
                        .get(&Resource::Guide)
                        .map(serde_json::to_value)
                        .transpose()?
            {
                context.revoke_guide();
            }
        }
        drop(sources);
        Ok(self.snapshot())
    }
    pub fn delete(&self, id: &str, revision: u64) -> Result<Value> {
        let _guard = self.operation.lock().expect("operation");
        let mut sources = self.sources.lock().expect("sources");
        if sources.get(id).is_none_or(|s| s.revision != revision) {
            bail!("La lista è cambiata. Riaprila prima di eliminarla.");
        }
        let mut next = sources.clone();
        next.remove(id);
        self.save(&next)?;
        *sources = next;
        self.revoke(id);
        self.cancel_source_imports(id, None);
        let _ = fs::remove_file(self.path(id)?);
        let _ = fs::remove_file(self.guide_path(id)?);
        let watch = self.watch_db()?;
        watch.execute("DELETE FROM watch WHERE source=?", [id])?;
        watch.execute("DELETE FROM video_saved WHERE source=?", [id])?;
        let logos = self.directory.join("logos").join(id);
        if logos.exists() {
            fs::remove_dir_all(logos)?;
        }
        drop(sources);
        Ok(self.snapshot())
    }
    fn source_error(&self, id: &str, message: &str, retry: u64) {
        let _guard = self.operation.lock().expect("operation");
        let mut sources = self.sources.lock().expect("sources");
        if let Some(source) = sources.get_mut(id) {
            source.error = message.into();
            source.retry_at = source.retry_at.max(retry);
            source.checked_at = now();
            let _ = self.save(&sources);
        }
    }
    pub fn refresh(self: &Arc<Self>, id: &str, force: bool) -> Result<Value> {
        let source = self.source(id)?;
        if source.url.is_none() {
            bail!("Sostituisci il file per aggiornare questa lista.");
        }
        if !source.enabled {
            bail!("Attiva la lista prima di aggiornarla.");
        }
        if source.retry_at > now() {
            bail!("Il servizio richiede una pausa prima del prossimo aggiornamento.");
        }
        let interval = if source.settings.refresh_minutes > 0 {
            source.settings.refresh_minutes
        } else if source.report.refresh_minutes > 0 {
            source.report.refresh_minutes
        } else {
            1440
        } as u64
            * 60000;
        if !force && now().saturating_sub(source.checked_at) < interval {
            return Ok(json!({"phase":"unchanged"}));
        }
        if self.jobs.lock().expect("jobs").values().any(|j| {
            j.config.source_id == id
                && !["done", "error", "cancelled"]
                    .contains(&j.status.lock().expect("status").phase.as_str())
        }) {
            return Ok(json!({"phase":"reading"}));
        }
        self.prepare(
            ImportConfig {
                source_id: source.id,
                revision: source.revision,
                name: source.name,
                url: source.url,
                settings: source.settings,
            },
            true,
        )
    }
    pub fn refresh_due(self: &Arc<Self>) {
        let sources = self
            .sources
            .lock()
            .expect("sources")
            .values()
            .filter(|s| s.enabled)
            .cloned()
            .collect::<Vec<_>>();
        for source in sources {
            if source.url.is_some() {
                let _ = self.refresh(&source.id, false);
            }
            if source.settings.guide.enabled {
                self.refresh_guide(&source.id, false);
            }
        }
    }
    pub(super) fn selected(&self, id: &str) -> Vec<Source> {
        self.sources
            .lock()
            .expect("sources")
            .values()
            .filter(|s| s.enabled && (id.is_empty() || s.id == id))
            .cloned()
            .collect()
    }
    fn filter(
        &self,
        db: &Connection,
        q: &CatalogQuery,
        source: &Source,
        favorites: &[String],
    ) -> Result<(String, Vec<String>)> {
        db.execute_batch("CREATE TEMP TABLE IF NOT EXISTS lume_hidden(id TEXT PRIMARY KEY);CREATE TEMP TABLE IF NOT EXISTS lume_order(id TEXT PRIMARY KEY,position INTEGER);")?;
        let layout = self.layout.lock().unwrap().clone();
        if let Some(rows) = layout["hidden"].as_array() {
            let mut insert = db.prepare("INSERT OR IGNORE INTO lume_hidden VALUES(?)")?;
            for r in rows {
                if let Some(id) = r["id"].as_str() {
                    insert.execute([id])?;
                }
            }
        }
        if let Some(rows) = layout["order"].as_array() {
            let mut insert = db.prepare("INSERT OR IGNORE INTO lume_order VALUES(?,?)")?;
            for (position, r) in rows.iter().enumerate() {
                if let Some(id) = r.as_str() {
                    insert.execute(params![id, position])?;
                }
            }
        }
        let mut clauses = vec![
            "active=1".to_owned(),
            "id NOT IN(SELECT id FROM lume_hidden)".into(),
        ];
        let mut args = vec![];
        for (column, value) in [("kind", &q.kind), ("content", &q.content)] {
            if !value.is_empty() {
                if column == "content" && source.settings.content != "auto" {
                    if value != &source.settings.content {
                        clauses.push("0".into());
                    }
                } else {
                    clauses.push(format!("{column}=?"));
                    args.push(value.clone());
                }
            }
        }
        if !q.query.is_empty() {
            clauses.push("(name LIKE ? ESCAPE '\\' OR CAST(number AS TEXT)=?)".into());
            args.push(format!(
                "%{}%",
                q.query
                    .chars()
                    .take(300)
                    .collect::<String>()
                    .replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_")
            ));
            args.push(q.query.chars().take(300).collect());
        }
        if !q.group.is_empty() {
            clauses.push("position IN(SELECT position FROM groups WHERE name=?)".into());
            args.push(q.group.clone());
        }
        if q.favorites {
            db.execute_batch("CREATE TEMP TABLE selected_ids(id TEXT PRIMARY KEY)")?;
            let mut insert = db.prepare("INSERT OR IGNORE INTO selected_ids VALUES(?)")?;
            for id in favorites {
                insert.execute([id])?;
            }
            clauses.push("id IN(SELECT id FROM selected_ids)".into());
        }
        Ok((clauses.join(" AND "), args))
    }
    fn display_rows(
        db: &Connection,
        sql: &str,
        args: &[String],
        source: &Source,
    ) -> Result<Vec<DisplayChannel>> {
        let mut statement = db.prepare(sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(args), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<u32>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, usize>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, bool>(9)?,
                r.get::<_, bool>(10)?,
            ))
        })?;
        let mut result = vec![];
        for row in rows {
            let (
                id,
                name,
                number,
                kind,
                content,
                position,
                groups,
                xml_id,
                tvg_name,
                logo_provided,
                guide_provided,
            ) = row?;
            let entry = Entry {
                id,
                name,
                number,
                kind: if kind == "radio" {
                    crate::model::MediaKind::Radio
                } else {
                    crate::model::MediaKind::Tv
                },
                content,
                groups: serde_json::from_str(&groups)?,
                logo: String::new(),
                url: String::new(),
                headers: Headers::new(),
                metadata: Metadata::default(),
                position,
            };
            let mut displayed = entry.display(&source.id, &source.settings);
            displayed.hints = super::metadata::Hints {
                xml_id,
                tvg_name,
                logo_provided,
                guide_provided,
            };
            result.push(displayed);
        }
        Ok(result)
    }
    pub fn catalog(&self, q: &CatalogQuery, favorites: &[String]) -> Result<Value> {
        let _guard = self.operation.lock().expect("operation");
        let mut rows = vec![];
        let mut total = 0;
        let mut skip = q.offset;
        let limit = if q.limit == 0 { 200 } else { q.limit.min(200) };
        for source in self.selected(&q.source) {
            let db = open(&self.path(&source.id)?)?;
            super::video::ensure_content(&db, &source.settings)?;
            let (filter, args) = self.filter(&db, q, &source, favorites)?;
            let count = db.query_row(
                &format!("SELECT COUNT(*) FROM records WHERE {filter}"),
                rusqlite::params_from_iter(&args),
                |r| r.get::<_, usize>(0),
            )?;
            total += count;
            if skip >= count {
                skip -= count;
                continue;
            }
            if rows.len() >= limit {
                continue;
            }
            let mut values = args;
            values.push((limit - rows.len()).to_string());
            values.push(skip.to_string());
            skip = 0;
            let mut page=Self::display_rows(&db,&format!("SELECT {DISPLAY_COLUMNS} FROM records WHERE {filter} ORDER BY COALESCE((SELECT position FROM lume_order WHERE lume_order.id=records.id),2147483647),position,id LIMIT ? OFFSET ?"),&values,&source)?;
            self.decorate_channels(&source, &mut page);
            rows.extend(page);
        }
        Ok(json!({"rows":rows,"total":total,"offset":q.offset}))
    }
    pub fn home(&self, source: &str, recent: &[String]) -> Result<Value> {
        let query = CatalogQuery {
            source: source.into(),
            kind: "tv".into(),
            content: "live".into(),
            limit: 1,
            ..Default::default()
        };
        let first = self.catalog(&query, &[])?;
        let recent = recent.iter().take(40).cloned().collect::<Vec<_>>();
        let mut rows = if recent.is_empty() {
            Vec::new()
        } else {
            // Reuse the bounded ID selection, source and content filters used
            // by favorites. This query selects the recent IDs, never all rows.
            self.catalog(
                &CatalogQuery {
                    favorites: true,
                    limit: 40,
                    ..query
                },
                &recent,
            )?["rows"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        };
        rows.sort_by_key(|channel| {
            recent
                .iter()
                .position(|id| channel["id"].as_str() == Some(id))
                .unwrap_or(usize::MAX)
        });
        rows.truncate(5);
        let featured = rows
            .first()
            .cloned()
            .unwrap_or_else(|| first["rows"][0].clone());
        Ok(json!({"total":first["total"],"featured":featured,"recent":rows}))
    }

    pub fn groups(&self, q: &CatalogQuery) -> Result<Value> {
        let _guard = self.operation.lock().expect("operation");
        let sources = self.selected(&q.source);
        if sources.is_empty() {
            return Ok(json!({"rows":[],"total":0}));
        }
        let db = open(&self.path(&sources[0].id)?)?;
        super::video::ensure_content(&db, &sources[0].settings)?;
        let _ = self.filter(&db, &CatalogQuery::default(), &sources[0], &[])?;
        let mut selects = vec![];
        let mut args = vec![];
        for (i, source) in sources.iter().enumerate() {
            let schema = if i == 0 {
                "main".into()
            } else {
                let name = format!("catalog{i}");
                db.execute(
                    &format!("ATTACH DATABASE ? AS {name}"),
                    [self.path(&source.id)?.to_string_lossy().as_ref()],
                )?;
                name
            };
            let mut filter = vec![
                "r.active=1".to_owned(),
                "r.id NOT IN(SELECT id FROM lume_hidden)".into(),
            ];
            if !q.kind.is_empty() {
                filter.push("r.kind=?".into());
                args.push(q.kind.clone());
            }
            if !q.content.is_empty() {
                if source.settings.content != "auto" {
                    if source.settings.content != q.content {
                        filter.push("0".into());
                    }
                } else {
                    filter.push("r.content=?".into());
                    args.push(q.content.clone());
                }
            }
            selects.push(format!("SELECT g.name FROM {schema}.groups g JOIN {schema}.records r ON r.position=g.position WHERE {}",filter.join(" AND ")));
        }
        let sql = format!(
            "SELECT DISTINCT name FROM ({}) WHERE name LIKE ? ESCAPE '\\'",
            selects.join(" UNION ALL ")
        );
        args.push(format!(
            "%{}%",
            q.query
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        ));
        let total: usize = db.query_row(
            &format!("SELECT COUNT(*) FROM ({sql})"),
            rusqlite::params_from_iter(&args),
            |r| r.get(0),
        )?;
        args.push("50".into());
        args.push(q.offset.to_string());
        let mut statement = db.prepare(&format!(
            "{sql} ORDER BY name COLLATE NOCASE,name LIMIT ? OFFSET ?"
        ))?;
        let rows = statement
            .query_map(rusqlite::params_from_iter(&args), |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(json!({"rows":rows,"total":total}))
    }
    pub(super) fn entry(&self, id: &str) -> Result<(Source, Entry)> {
        if !id.starts_with("personal_") || !crate::catalog::valid_id(id) {
            bail!("Canale personale non valido.");
        }
        let _guard = self.operation.lock().expect("operation");
        for source in self.selected("") {
            let db = open(&self.path(&source.id)?)?;
            super::video::ensure_content(&db, &source.settings)?;
            let payload = db
                .query_row(
                    "SELECT payload,position FROM records WHERE id=? AND active=1",
                    [id],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, usize>(1)?)),
                )
                .optional()?;
            if let Some((payload, position)) = payload {
                let mut entry: Entry = serde_json::from_str(&payload)?;
                entry.content = super::video::classify(&entry, &source.settings).into();
                entry.id = id.into();
                entry.position = position;
                return Ok((source, entry));
            }
        }
        bail!("Il canale non è più disponibile nella lista.")
    }
    pub fn channel(&self, id: &str) -> Option<DisplayChannel> {
        if self.is_hidden(id) {
            return None;
        }
        let (source, entry) = self.entry(id).ok()?;
        let mut channel = entry.display(&source.id, &source.settings);
        let _operation = self.operation.lock().ok()?;
        self.decorate_channels(&source, std::slice::from_mut(&mut channel));
        Some(channel)
    }
    pub(super) fn decorate(db: &Connection, channel: &mut DisplayChannel) {
        if channel.content != "live" || channel.channel.kind != crate::model::MediaKind::Tv {
            return;
        }
        let time = now();
        for (current, target) in [(true, &mut channel.now), (false, &mut channel.next)] {
            let filter = if current {
                "start<=?2 AND end>?2 ORDER BY start DESC"
            } else {
                "start>?2 ORDER BY start"
            };
            *target=db.query_row(&format!("SELECT title,start,end,description FROM programmes WHERE channel=?1 AND {filter} LIMIT 1"),params![channel.channel.id,time],|r|Ok(crate::guide::Programme{title:r.get(0)?,start:r.get(1)?,end:r.get(2)?,description:r.get(3)?,category:String::new()})).optional().ok().flatten();
        }
    }
    pub fn next(
        &self,
        id: &str,
        direction: i32,
        favorites: &[String],
        only: bool,
    ) -> Result<Option<DisplayChannel>> {
        let (source, entry) = self.entry(id)?;
        let _guard = self.operation.lock().expect("operation");
        let db = open(&self.path(&source.id)?)?;
        super::video::ensure_content(&db, &source.settings)?;
        let q = CatalogQuery {
            kind: if entry.kind == crate::model::MediaKind::Radio {
                "radio"
            } else {
                "tv"
            }
            .into(),
            content: "live".into(),
            favorites: only,
            ..Default::default()
        };
        let (filter, args) = self.filter(&db, &q, &source, favorites)?;
        let rank: i64 = db.query_row(
            "SELECT COALESCE((SELECT position FROM lume_order WHERE id=?),2147483647)",
            [id],
            |r| r.get(0),
        )?;
        for wrap in [false, true] {
            let mut args = args.clone();
            let boundary = if wrap {
                String::new()
            } else {
                args.push(rank.to_string());
                args.push(entry.position.to_string());
                args.push(id.into());
                format!(" AND (COALESCE((SELECT position FROM lume_order WHERE lume_order.id=records.id),2147483647),position,id) {} (CAST(? AS INTEGER),CAST(? AS INTEGER),?)",if direction>=0{">"}else{"<"})
            };
            let sql=format!("SELECT {DISPLAY_COLUMNS} FROM records WHERE {filter}{boundary} ORDER BY COALESCE((SELECT position FROM lume_order WHERE lume_order.id=records.id),2147483647) {order},position {order},id {order} LIMIT 1",order=if direction>=0{"ASC"}else{"DESC"});
            if let Some(mut channel) = Self::display_rows(&db, &sql, &args, &source)?
                .into_iter()
                .next()
            {
                self.decorate_channels(&source, std::slice::from_mut(&mut channel));
                return Ok(Some(channel));
            }
        }
        Ok(None)
    }
    pub fn logo(&self, id: &str) -> Result<(String, Vec<u8>)> {
        let (source, entry) = self.entry(id)?;
        self.logo_url_headers(&source, &entry.logo, &entry.headers)
    }
    pub(super) fn logo_url_headers(
        &self,
        source: &Source,
        url: &str,
        entry_headers: &Headers,
    ) -> Result<(String, Vec<u8>)> {
        let headers = entry_headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("user-agent"))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<Headers>();
        if url.is_empty() {
            bail!("Logo non disponibile.");
        }
        let context = self.context(source)?;
        let directory = self.directory.join("logos").join(&source.id);
        private_directory(&directory)?;
        let key = format!(
            "{:x}",
            Sha256::digest(format!(
                "{}|{}|{}|{}",
                source.revision,
                url,
                serde_json::to_string(&headers)?,
                serde_json::to_string(&source.settings.access)?
            ))
        );
        let path = directory.join(key);
        let sniff = |bytes: &[u8]| -> Option<&'static str> {
            if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                Some("image/png")
            } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
                Some("image/jpeg")
            } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
                Some("image/webp")
            } else {
                None
            }
        };
        if let Ok(bytes) = fs::read(&path) {
            context.check()?;
            if bytes.len() <= 1024 * 1024 {
                if let Some(mime) = sniff(&bytes) {
                    return Ok((mime.into(), bytes));
                }
            }
        }
        let mut downloads = self.logo_downloads.lock().expect("logo downloads");
        while *downloads >= 3 {
            context.check()?;
            downloads = self
                .logo_ready
                .wait_timeout(downloads, std::time::Duration::from_millis(100))
                .expect("logo downloads")
                .0;
        }
        context.check()?;
        *downloads += 1;
        drop(downloads);
        let _slot = LogoSlot(self);
        let response = context.request(url, url, Resource::Logo, &headers, Method::GET, None)?;
        if !response.status().is_success() {
            bail!("Logo non disponibile.");
        }
        let mut bytes = vec![];
        response.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 1024 * 1024 {
            bail!("Logo troppo grande.");
        }
        let mime = sniff(&bytes)
            .ok_or_else(|| anyhow!("Formato del logo non supportato."))?
            .to_owned();
        context.check()?;
        let mut file = fs::File::create(&path)?;
        private_file(&path)?;
        file.write_all(&bytes)?;
        let mut files = fs::read_dir(&directory)?
            .filter_map(|f| f.ok())
            .filter_map(|f| {
                f.metadata()
                    .ok()
                    .map(|m| (f.path(), m.len(), m.modified().ok()))
            })
            .collect::<Vec<_>>();
        let mut total = files.iter().map(|(_, n, _)| *n).sum::<u64>();
        if files.len() > 1024 || total > 24 * 1024 * 1024 {
            files.sort_by_key(|(_, _, modified)| *modified);
            let mut count = files.len();
            for (path, size, _) in files {
                if count <= 768 && total <= 20 * 1024 * 1024 {
                    break;
                }
                if fs::remove_file(path).is_ok() {
                    count -= 1;
                    total = total.saturating_sub(size);
                }
            }
        }
        Ok((mime, bytes))
    }
    pub(super) fn watch_db(&self) -> Result<Connection> {
        let db = open(&self.directory.join("watch.sqlite"))?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS watch(source TEXT,channel TEXT,mode TEXT,event TEXT,position REAL,duration REAL,completed INTEGER,updated INTEGER,PRIMARY KEY(source,channel,mode,event));")?;
        let hidden = db
            .prepare("PRAGMA table_info(watch)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<std::result::Result<Vec<_>, _>>()?
            .iter()
            .any(|name| name == "hidden");
        if !hidden {
            db.execute_batch("ALTER TABLE watch ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0;")?;
        }
        let revision = db
            .prepare("PRAGMA table_info(watch)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<std::result::Result<Vec<_>, _>>()?
            .iter()
            .any(|name| name == "revision");
        if !revision {
            db.execute_batch("ALTER TABLE watch ADD COLUMN revision INTEGER NOT NULL DEFAULT 0;")?;
        }
        db.execute_batch("CREATE INDEX IF NOT EXISTS watch_recent ON watch(mode,completed,updated DESC); CREATE TABLE IF NOT EXISTS video_saved(source TEXT NOT NULL,key TEXT NOT NULL,PRIMARY KEY(source,key));")?;
        Ok(db)
    }
    pub fn watch(
        &self,
        id: &str,
        mode: &str,
        event: &str,
        position: f64,
        duration: f64,
        completed: bool,
    ) -> Result<()> {
        self.watch_checked(id, mode, event, position, duration, completed, None)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn watch_checked(
        &self,
        id: &str,
        mode: &str,
        event: &str,
        position: f64,
        duration: f64,
        completed: bool,
        revision: Option<u64>,
    ) -> Result<()> {
        if !["vod", "archive"].contains(&mode)
            || event.len() > 100
            || !position.is_finite()
            || !duration.is_finite()
            || position < 0.0
            || duration < 0.0
        {
            bail!("Posizione del video non valida.");
        }
        let (source, _) = self.entry(id)?;
        let _guard = self.operation.lock().expect("operation");
        let current = self.source(&source.id)?;
        if current.revision != source.revision || current.snapshot != source.snapshot {
            return Ok(());
        }
        let db = self.watch_db()?;
        if let Some(revision) = revision {
            if db.query_row("SELECT revision FROM watch WHERE source=? AND channel=? AND mode=? AND event=?",params![source.id,id,mode,event],|r|r.get::<_,u64>(0)).optional()? != Some(revision) {return Ok(());}
        }
        db.execute(
            "INSERT INTO watch(source,channel,mode,event,position,duration,completed,updated) VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(source,channel,mode,event) DO UPDATE SET position=excluded.position,duration=excluded.duration,completed=excluded.completed,updated=excluded.updated,hidden=0",
            params![
                source.id,
                id,
                mode,
                event,
                position,
                duration,
                completed,
                now()
            ],
        )?;
        db.execute_batch("DELETE FROM watch WHERE rowid NOT IN(SELECT rowid FROM watch ORDER BY updated DESC LIMIT 5000)")?;
        Ok(())
    }
    pub fn playback(
        &self,
        id: &str,
        mode: &str,
        event: &str,
        position: Option<f64>,
    ) -> Result<PersonalPlayback> {
        if self.is_hidden(id) {
            bail!("Il canale è nascosto. Ripristinalo dalle impostazioni.");
        }
        let (source, mut entry) = self.entry(id)?;
        let mut display = entry.display(&source.id, &source.settings);
        self.decorate_channels(&source, std::slice::from_mut(&mut display));
        let context = self.context(&source)?;
        let mode = if mode == "archive" {
            "archive"
        } else if display.content == "vod" {
            "vod"
        } else {
            "live"
        };
        let (saved, watch_revision) = {
            let _operation = self.operation.lock().expect("watch start");
            let current = self.source(&source.id)?;
            if !current.enabled
                || current.revision != source.revision
                || current.snapshot != source.snapshot
            {
                bail!("La lista è cambiata. Riapri il video.");
            }
            let db = self.watch_db()?;
            let saved=db.query_row("SELECT position,completed FROM watch WHERE source=? AND channel=? AND mode=? AND event=?",params![source.id,id,mode,event],|r|Ok((r.get::<_,f64>(0)?,r.get::<_,bool>(1)?))).optional()?.filter(|(p,c)|*p>=5.0&&!*c).map(|(p,_)|p).unwrap_or(0.0);
            let mut revision = 0;
            if mode != "live" {
                let start = position
                    .filter(|p| p.is_finite() && *p >= 0.0)
                    .unwrap_or(saved);
                db.execute("INSERT INTO watch(source,channel,mode,event,position,duration,completed,updated,revision) VALUES(?,?,?,?,?,?,0,?,1) ON CONFLICT(source,channel,mode,event) DO UPDATE SET position=excluded.position,completed=0,hidden=0,updated=excluded.updated,revision=revision+1",params![source.id,id,mode,event,start,entry.metadata.duration.max(0.0),now()])?;
                revision=db.query_row("SELECT revision FROM watch WHERE source=? AND channel=? AND mode=? AND event=?",params![source.id,id,mode,event],|r|r.get(0))?;
            }
            (saved, revision)
        };
        let mut position = position.unwrap_or(saved);
        if !position.is_finite() || position < 0.0 {
            position = 0.0;
        }
        let provider = super::localplay::route(&entry)?;
        if provider.is_some() && mode == "archive" {
            bail!("L’archivio non è disponibile per questo collegamento localplay.");
        }
        let resolved = super::localplay::resolve(&entry)?;
        context.check()?;
        let root = entry.url.clone();
        let mut title = entry.name.clone();
        let mut duration = entry.metadata.duration.max(0.0);
        let mut base_offset = 0.0;
        if mode == "archive" {
            let programme = self.guide_event(&source.id, id, event)?;
            let archive =
                catchup::build(&entry, &source.settings.guide, &programme, now(), position)?;
            entry.url = archive.url;
            duration = archive.duration;
            title = programme.programme.title;
            if archive.regenerate {
                base_offset = archive.offset;
                position = 0.0;
            }
        }
        let (prepared, mut policy) =
            super::localplay::configure(&entry, context, resolved.as_ref())?;
        entry = prepared;
        if mode == "live" {
            if let Some(url) = super::hls::representation(&entry, &policy)? {
                entry.url = url;
            }
        }
        if mode == "archive" {
            Arc::get_mut(&mut policy).expect("new policy").root = root;
        }
        let hint = entry
            .metadata
            .properties
            .get("inputstream.adaptive.manifest_type")
            .map(String::as_str)
            .unwrap_or("");
        let path = reqwest::Url::parse(&entry.url)?.path().to_lowercase();
        let mut mime = if hint == "hls" || path.ends_with(".m3u8") {
            Some("application/x-mpegURL".to_owned())
        } else if ["mpd", "dash"].contains(&hint) || path.ends_with(".mpd") {
            Some("application/dash+xml".into())
        } else if path.ends_with(".mp4") || path.ends_with(".m4v") {
            Some("video/mp4".into())
        } else if path.ends_with(".webm") {
            Some("video/webm".into())
        } else if path.ends_with(".mp3") {
            Some("audio/mpeg".into())
        } else if path.ends_with(".aac") {
            Some("audio/aac".into())
        } else {
            None
        };
        if mime.is_none() {
            let response = policy.media_request(&entry.url, Resource::Manifest, &Headers::new())?;
            if !response.status().is_success() {
                bail!("Accesso al contenuto negato.");
            }
            mime = response
                .headers()
                .get("Content-Type")
                .and_then(|v| v.to_str().ok())
                .map(|v| v.split(';').next().unwrap_or("").into());
            let mut beginning = Vec::new();
            response.take(2048).read_to_end(&mut beginning)?;
            let text = String::from_utf8_lossy(&beginning);
            if text.trim_start().starts_with("#EXTM3U") {
                mime = Some("application/x-mpegURL".into());
            } else if text.contains("<MPD") {
                mime = Some("application/dash+xml".into());
            }
        }
        if policy.key_system() == Some("org.w3.clearkey")
            && mime.as_deref() != Some("application/dash+xml")
        {
            bail!("ClearKey personale richiede DASH con cifratura CENC.");
        }
        let seekable = !entry
            .metadata
            .properties
            .get("lumetv.pipe_headers")
            .is_some_and(|h| {
                reqwest::Url::parse(&format!("https://options.invalid/?{h}"))
                    .is_ok_and(|u| u.query_pairs().any(|(k, v)| k == "seekable" && v == "0"))
            });
        let stream = ResolvedStream {
            url: entry.url,
            mime_type: mime,
            headers: policy.media.clone(),
            drm: policy.drm.as_ref().map(|d| crate::model::DrmSpec {
                license_url: d.url.clone(),
                headers: d.headers.clone(),
            }),
        };
        Ok(PersonalPlayback {
            stream,
            policy,
            channel: display,
            mode: mode.into(),
            title,
            position,
            duration,
            base_offset,
            event: event.into(),
            seekable,
            watch_revision,
        })
    }
}
