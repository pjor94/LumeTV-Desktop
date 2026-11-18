// SPDX-License-Identifier: GPL-3.0-or-later
//! Android companion sender: pinned local TLS, pairing, remote, watch handoff and playlist transfer.
mod discovery;
#[cfg(test)]
mod tests;
mod transport;
use crate::personal::Library;
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Digest;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use transport::Connection;
#[derive(Clone, Serialize, Deserialize)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub id: String,
    #[serde(default = "tv_name")]
    pub name: String,
}
fn tv_name() -> String {
    "TV".into()
}
impl Endpoint {
    fn validate(&self) -> Result<()> {
        if !local_host(&self.host) || self.port == 0 || !self.id.is_empty() && !uuid(&self.id) {
            bail!("Indica l’indirizzo IPv4 privato e la porta mostrati da Lume sulla TV.");
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Television {
    endpoint: Endpoint,
    pin: String,
    token: String,
}
#[derive(Serialize, Deserialize)]
struct Store {
    id: String,
    tvs: Vec<Television>,
}
struct Job {
    id: String,
    tv: String,
    kind: String,
    status: Mutex<Value>,
    cancelled: AtomicBool,
    busy: AtomicBool,
    reference: Mutex<Option<Value>>,
}
impl Job {
    fn check(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Acquire) {
            bail!("Operazione annullata.");
        }
        Ok(())
    }
    fn set(&self, value: Value) {
        *self.status.lock().unwrap() = value;
    }
    fn fail(&self, error: anyhow::Error) {
        self.set(json!({"id":self.id,"phase":if self.cancelled.load(Ordering::Acquire){"cancelled"}else{"error"},"error":error.to_string()}));
    }
}
pub struct Companion {
    directory: PathBuf,
    store: Mutex<Store>,
    jobs: Mutex<BTreeMap<String, Arc<Job>>>,
}
pub(crate) fn local_host(value: &str) -> bool {
    let p = value.split('.').collect::<Vec<_>>();
    if p.len() != 4
        || p.iter().any(|s| {
            s.is_empty()
                || s.len() > 3
                || s.len() > 1 && s.starts_with('0')
                || !s.bytes().all(|b| b.is_ascii_digit())
        })
    {
        return false;
    }
    let Some(p) = p
        .iter()
        .map(|s| s.parse::<u8>().ok())
        .collect::<Option<Vec<_>>>()
    else {
        return false;
    };
    p[3] != 0
        && p[3] != 255
        && (p[0] == 10
            || p[0] == 192 && p[1] == 168
            || p[0] == 172 && (16..=31).contains(&p[1])
            || p[0] == 169 && p[1] == 254)
}
fn uuid(s: &str) -> bool {
    uuid::Uuid::parse_str(s).is_ok_and(|u| u.to_string() == s)
}
fn hex(s: &str, n: usize) -> bool {
    s.len() == n * 2
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn random() -> Result<String> {
    use ring::rand::{SecureRandom, SystemRandom};
    let mut b = [0; 32];
    SystemRandom::new()
        .fill(&mut b)
        .map_err(|_| anyhow!("Impossibile preparare l’associazione."))?;
    Ok(b.iter().map(|b| format!("{b:02x}")).collect())
}
fn same(a: &str, b: &str) -> bool {
    use sha2::Sha256;
    let a = Sha256::digest(a.as_bytes());
    let b = Sha256::digest(b.as_bytes());
    ring::hmac::verify(
        &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &a),
        b"lume-compare",
        ring::hmac::sign(
            &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &b),
            b"lume-compare",
        )
        .as_ref(),
    )
    .is_ok()
}
fn display(value: &str, max: usize) -> String {
    static URL: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)https?://\S+").unwrap());
    URL.replace_all(value, "…")
        .chars()
        .filter(|c| {
            !c.is_control()
                && !('\u{202a}'..='\u{202e}').contains(c)
                && !('\u{2066}'..='\u{2069}').contains(c)
        })
        .take(max)
        .collect::<String>()
        .trim()
        .to_owned()
}
fn proof(
    code: &str,
    role: &str,
    pin: &str,
    client: &str,
    server: &str,
    id: &str,
    name: &str,
) -> String {
    let value = ["lume-companion-v1", role, pin, client, server, id, name].join("\0");
    ring::hmac::sign(
        &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, code.as_bytes()),
        value.as_bytes(),
    )
    .as_ref()
    .iter()
    .map(|b| format!("{b:02x}"))
    .collect()
}
fn message(code: &str) -> String {
    match code {"pairing_failed"=>"Il codice di associazione non corrisponde alla TV.","pairing_closed"|"pairing_expired"=>"Il codice è scaduto. Riapri l’associazione sulla TV.","pairing_busy"|"handover_pending"=>"La TV sta completando un’altra associazione.","slow_down"=>"Troppi tentativi. Attendi un minuto e riprova.","pairing_denied"=>"Associazione rifiutata sulla TV.","not_authorized"=>"Associazione revocata. Associa di nuovo la TV.","watch_missing"=>"Il contenuto non è presente sulla TV. Per una lista personale, invia prima la lista alla TV.","watch_ambiguous"=>"Più contenuti sulla TV corrispondono a questa voce. Controlla le liste sulla TV.","watch_tv_changed"|"playback_changed"|"playback_changing"=>"La riproduzione sulla TV è cambiata. Aggiorna e riprova.","watch_not_seekable"=>"La TV non può riprendere questo video dal punto salvato.","watch_playback_failed"=>"La TV ha trovato il contenuto ma non riesce a riprodurlo.","tracks_changed"=>"Le tracce sulla TV sono cambiate. Aggiorna e riprova.","watch_timeout"=>"La TV non ha confermato l’avvio in tempo. Verifica la riproduzione sulla TV.","watch_unsupported"=>"Aggiorna Lume sulla TV per usare questa funzione.","playlist_busy"|"playlist_importing"=>"La TV sta già importando una lista.",_=>"La TV non può completare l’operazione. Aggiorna lo stato e riprova."}.into()
}
fn private(_path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(_path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
impl Companion {
    pub fn new(root: &Path) -> Result<Arc<Self>> {
        let directory = root.join("companion");
        fs::create_dir_all(&directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        }
        let path = directory.join("devices.json");
        let store = if path.is_file() {
            let bytes = fs::read(&path)?;
            if bytes.len() > 32768 {
                bail!("Archivio TV non valido.");
            }
            let s: Store = serde_json::from_slice(&bytes)?;
            if !uuid(&s.id)
                || s.tvs.len() > 8
                || s.tvs.iter().any(|t| {
                    t.endpoint.validate().is_err()
                        || !hex(&t.pin, 32)
                        || !hex(&t.token, 32)
                        || !uuid(&t.endpoint.id)
                })
            {
                bail!("Associazioni TV non valide.");
            }
            s
        } else {
            Store {
                id: uuid::Uuid::new_v4().to_string(),
                tvs: Vec::new(),
            }
        };
        for f in fs::read_dir(&directory)? {
            let f = f?;
            if f.file_name().to_string_lossy().starts_with("transfer-") {
                let _ = fs::remove_file(f.path());
            }
        }
        Ok(Arc::new(Self {
            directory,
            store: Mutex::new(store),
            jobs: Mutex::new(BTreeMap::new()),
        }))
    }
    fn save(&self, store: &Store) -> Result<()> {
        let temp = self.directory.join("devices.tmp");
        let mut f = File::create(&temp)?;
        private(&temp)?;
        f.write_all(&serde_json::to_vec(store)?)?;
        f.sync_all()?;
        fs::rename(temp, self.directory.join("devices.json"))?;
        Ok(())
    }
    pub fn devices(&self) -> Value {
        json!({"televisions":self.store.lock().unwrap().tvs.iter().map(|t|&t.endpoint).collect::<Vec<_>>()})
    }
    pub fn discover(&self) -> Result<Value> {
        Ok(json!({"televisions":discovery::discover()?}))
    }
    fn connection(&self, id: &str) -> Result<Connection> {
        let tv = self
            .store
            .lock()
            .unwrap()
            .tvs
            .iter()
            .find(|t| t.endpoint.id == id)
            .cloned()
            .ok_or_else(|| anyhow!("Scegli una TV associata."))?;
        Connection::new(tv.endpoint, Some(tv.pin), Some(tv.token))
    }
    fn job(&self, id: &str) -> Result<Arc<Job>> {
        self.jobs
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow!("Operazione non disponibile. Riprova."))
    }
    fn new_job(&self, tv: &str, kind: &str) -> Result<Arc<Job>> {
        let mut jobs = self.jobs.lock().unwrap();
        jobs.retain(|_, j| {
            j.busy.load(Ordering::Acquire)
                || (!j.cancelled.load(Ordering::Acquire)
                    && !["paired", "imported", "started"]
                        .contains(&j.status.lock().unwrap()["phase"].as_str().unwrap_or("")))
        });
        if jobs.len() >= 8 {
            bail!("Chiudi un’operazione prima di continuare.");
        }
        let id = uuid::Uuid::new_v4().to_string();
        let j = Arc::new(Job {
            id: id.clone(),
            tv: tv.into(),
            kind: kind.into(),
            status: Mutex::new(json!({"id":id,"phase":"working","error":""})),
            cancelled: AtomicBool::new(false),
            busy: AtomicBool::new(false),
            reference: Mutex::new(None),
        });
        jobs.insert(id, j.clone());
        Ok(j)
    }
    pub fn status(&self, id: &str) -> Result<Value> {
        Ok(self.job(id)?.status.lock().unwrap().clone())
    }
    pub fn forget(&self, id: &str) -> Result<Value> {
        let mut store = self.store.lock().unwrap();
        store.tvs.retain(|t| t.endpoint.id != id);
        self.save(&store)?;
        drop(store);
        for j in self.jobs.lock().unwrap().values().filter(|j| j.tv == id) {
            j.cancelled.store(true, Ordering::Release);
        }
        Ok(self.devices())
    }
    pub fn state(&self, id: &str) -> Result<Value> {
        let value = self.connection(id)?.exchange("/v1/state", None, true)?;
        if value["version"] != 1 || value["session"].as_str().is_none_or(|s| s.len() > 128) {
            bail!("Aggiorna Lume sulla TV.");
        }
        Ok(value)
    }
    pub fn command(&self, id: &str, value: &Value) -> Result<Value> {
        let action = value["action"].as_str().unwrap_or("");
        if ![
            "navigate",
            "play",
            "pause",
            "stop",
            "channelNext",
            "channelPrevious",
            "seekBy",
            "seek",
            "live",
            "nextEpisode",
            "track",
            "subtitlesOff",
            "sleep",
        ]
        .contains(&action)
        {
            bail!("Comando TV non valido.");
        }
        let session = value["session"]
            .as_str()
            .filter(|s| s.len() <= 128)
            .ok_or_else(|| anyhow!("Aggiorna lo stato della TV."))?;
        let command = value["command_id"]
            .as_str()
            .filter(|s| uuid(s))
            .map(str::to_owned)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let body = json!({"id":command,"session":session,"action":action,"args":value.get("args").cloned().unwrap_or_else(||json!({}))});
        self.connection(id)?
            .exchange("/v1/command", Some(&body), true)
    }
    pub fn pair(self: &Arc<Self>, value: &Value) -> Result<Value> {
        let (endpoint, pin, code) =
            if let Some(invite) = value["invite"].as_str().filter(|s| !s.trim().is_empty()) {
                let p = invite.trim().split('|').collect::<Vec<_>>();
                if p.len() != 6 || p[0] != "LUME1" || invite.len() > 512 || !hex(p[4], 32) {
                    bail!("Invito Lume non valido.");
                }
                (
                    Endpoint {
                        host: p[1].into(),
                        port: p[2].parse()?,
                        id: p[3].into(),
                        name: tv_name(),
                    },
                    Some(p[4].to_string()),
                    p[5].to_string(),
                )
            } else {
                (
                    serde_json::from_value::<Endpoint>(value["endpoint"].clone())?,
                    None,
                    value["code"].as_str().unwrap_or("").to_owned(),
                )
            };
        endpoint.validate()?;
        let code = code
            .to_ascii_uppercase()
            .chars()
            .filter(|c| *c != ' ' && *c != '-')
            .collect::<String>();
        if code.len() != 16
            || !code
                .chars()
                .all(|c| "23456789ABCDEFGHJKLMNPQRSTUVWXYZ".contains(c))
        {
            bail!("Inserisci il codice di 16 caratteri mostrato sulla TV.");
        }
        let j = self.new_job(&endpoint.id, "pair")?;
        j.busy.store(true, Ordering::Release);
        let id = j.id.clone();
        let this = self.clone();
        thread::spawn(move || {
            let result = this.pair_worker(&j, endpoint, pin, code);
            if let Err(e) = result {
                j.fail(e);
            }
            j.busy.store(false, Ordering::Release);
        });
        self.status(&id)
    }
    fn pair_worker(
        &self,
        j: &Job,
        endpoint: Endpoint,
        pin: Option<String>,
        mut code: String,
    ) -> Result<()> {
        let connection = Connection::new(endpoint.clone(), pin, None)?;
        let client = random()?;
        let reply =
            connection.exchange("/v1/pair/challenge", Some(&json!({"nonce":client})), false)?;
        j.check()?;
        let pin = connection.pin()?;
        let server = reply["nonce"]
            .as_str()
            .filter(|s| hex(s, 32))
            .ok_or_else(|| anyhow!("Risposta di associazione non valida."))?;
        let tv_id = reply["id"]
            .as_str()
            .filter(|s| uuid(s))
            .ok_or_else(|| anyhow!("Identità TV non valida."))?;
        if reply["version"] != 1
            || !endpoint.id.is_empty() && endpoint.id != tv_id
            || !same(
                &proof(&code, "server", &pin, &client, server, tv_id, ""),
                reply["proof"].as_str().unwrap_or(""),
            )
        {
            bail!("Il codice non corrisponde all’identità della TV.");
        }
        let name = display(reply["name"].as_str().unwrap_or("TV"), 80);
        let client_id = self.store.lock().unwrap().id.clone();
        let reply=connection.exchange("/v1/pair/request",Some(&json!({"challenge":reply["challenge"],"clientId":client_id,"name":"Lume Desktop","proof":proof(&code,"client",&pin,&client,server,&client_id,"Lume Desktop")})),false)?;
        // The invitation secret is never persisted and is no longer needed after the proof.
        unsafe { code.as_bytes_mut().fill(0) };
        let ticket = reply["ticket"]
            .as_str()
            .filter(|s| hex(s, 32))
            .ok_or_else(|| anyhow!("Associazione non valida."))?;
        j.set(json!({"id":j.id,"phase":"waiting","name":name,"error":""}));
        let until = Instant::now() + Duration::from_secs(90);
        while Instant::now() < until {
            j.check()?;
            let status =
                connection.exchange("/v1/pair/status", Some(&json!({"ticket":ticket})), false)?;
            match status["status"].as_str() {
                Some("approved") => {
                    let token = status["token"]
                        .as_str()
                        .filter(|s| hex(s, 32))
                        .ok_or_else(|| anyhow!("Associazione non valida."))?;
                    let tv = Television {
                        endpoint: Endpoint {
                            id: tv_id.into(),
                            name,
                            ..endpoint
                        },
                        pin,
                        token: token.into(),
                    };
                    j.check()?;
                    let mut store = self.store.lock().unwrap();
                    if store.tvs.len() >= 8 && !store.tvs.iter().any(|t| t.endpoint.id == tv_id) {
                        bail!("Sono già associate otto TV. Rimuovine una prima di continuare.");
                    }
                    store.tvs.retain(|t| t.endpoint.id != tv_id);
                    store.tvs.push(tv);
                    self.save(&store)?;
                    drop(store);
                    let _ = self.state(tv_id);
                    j.set(json!({"id":j.id,"phase":"paired","television":tv_id,"error":""}));
                    return Ok(());
                }
                Some("denied") => bail!("Associazione rifiutata sulla TV."),
                Some("waiting") => {}
                _ => bail!("Associazione non valida."),
            };
            thread::sleep(Duration::from_millis(700));
        }
        bail!("Associazione scaduta. Riapri il codice sulla TV.")
    }
    pub fn resolve(self: &Arc<Self>, tv: &str, reference: Value) -> Result<Value> {
        let connection = self.connection(tv)?;
        if reference["reference"].is_null() {
            bail!("Contenuto non disponibile.");
        }
        let j = self.new_job(tv, "watch")?;
        *j.reference.lock().unwrap() = Some(reference.clone());
        let id = j.id.clone();
        j.busy.store(true, Ordering::Release);
        thread::spawn(move || {
            let result = (|| {
                let state = connection.exchange("/v1/state", None, true)?;
                if state["watchSupported"] != true {
                    bail!("Aggiorna Lume sulla TV per inviare un contenuto.");
                }
                let result = connection.exchange(
                    "/v1/watch/resolve",
                    Some(&json!({"id":j.id,"reference":reference["reference"]})),
                    true,
                )?;
                wait_watch(&j, &connection, result, false)
            })();
            if let Err(e) = result {
                j.fail(e);
            }
            j.busy.store(false, Ordering::Release);
        });
        self.status(&id)
    }
    pub fn start_watch(
        self: &Arc<Self>,
        id: &str,
        current: &Value,
        position: u64,
    ) -> Result<Value> {
        let j = self.job(id)?;
        let connection = self.connection(&j.tv)?;
        if j.kind != "watch"
            || j.status.lock().unwrap()["phase"] != "ready"
            || j.busy.swap(true, Ordering::AcqRel)
        {
            bail!("Verifica prima il contenuto sulla TV.");
        }
        let reference = j.reference.lock().unwrap().clone();
        if reference.as_ref() != Some(current) {
            j.busy.store(false, Ordering::Release);
            bail!("La lista sul computer è cambiata. Verifica di nuovo il contenuto sulla TV.");
        }
        if position > 30 * 24 * 60 * 60 * 1000
            || current["reference"]["vod"] != true && position != 0
        {
            j.busy.store(false, Ordering::Release);
            bail!("Posizione video non valida.");
        }
        j.set(json!({"id":j.id,"phase":"starting","error":""}));
        let id = j.id.clone();
        thread::spawn(move || {
            let result = (|| {
                j.check()?;
                let result = connection.exchange(
                    "/v1/watch/start",
                    Some(&json!({"id":j.id,"position":position})),
                    true,
                )?;
                wait_watch(&j, &connection, result, true)
            })();
            if let Err(e) = result {
                j.fail(e);
            }
            j.busy.store(false, Ordering::Release);
        });
        self.status(&id)
    }
    pub fn transfer(
        self: &Arc<Self>,
        tv: &str,
        library: Arc<Library>,
        source: &str,
    ) -> Result<Value> {
        let connection = self.connection(tv)?;
        let j = self.new_job(tv, "playlist")?;
        let id = j.id.clone();
        j.busy.store(true, Ordering::Release);
        let path = self.directory.join(format!("transfer-{id}.gz"));
        let source = source.to_owned();
        thread::spawn(move || {
            let result = (|| {
                let metadata = library.companion_transfer(&source, &path, &|| j.check())?;
                let size = fs::metadata(&path)?.len();
                let mut digest = sha2::Sha256::new();
                let mut file = File::open(&path)?;
                let mut buffer = [0; 16384];
                loop {
                    j.check()?;
                    let n = file.read(&mut buffer)?;
                    if n == 0 {
                        break;
                    }
                    digest.update(&buffer[..n]);
                }
                let hash = format!("{:x}", digest.finalize());
                let mut state = connection.exchange(
                    "/v1/playlists/start",
                    Some(&json!({"id":j.id,"name":metadata["name"],"size":size,"sha256":hash})),
                    true,
                )?;
                if state["status"] == "uploading" {
                    use base64::Engine;
                    use std::io::{Seek, SeekFrom};
                    let mut file = File::open(&path)?;
                    let mut offset = state["received"].as_u64().unwrap_or(0);
                    if offset > size {
                        bail!("Stato del trasferimento non valido.");
                    }
                    file.seek(SeekFrom::Start(offset))?;
                    while offset < size {
                        j.check()?;
                        let n = file.read(&mut buffer)?;
                        if n == 0 {
                            bail!("Trasferimento interrotto.");
                        }
                        state=connection.exchange("/v1/playlists/chunk",Some(&json!({"id":j.id,"offset":offset,"data":base64::engine::general_purpose::STANDARD.encode(&buffer[..n])})),true)?;
                        offset += n as u64;
                        if state["received"].as_u64() != Some(offset) {
                            bail!("La TV non ha confermato la ricezione della lista.");
                        }
                        j.set(json!({"id":j.id,"phase":"uploading","received":offset,"size":size,"error":""}));
                    }
                    state = connection.exchange(
                        "/v1/playlists/prepare",
                        Some(&json!({"id":j.id})),
                        true,
                    )?;
                }
                wait_playlist(&j, &connection, state)?;
                Ok(())
            })();
            let _ = fs::remove_file(path);
            if let Err(e) = result {
                let _ =
                    connection.exchange("/v1/playlists/cancel", Some(&json!({"id":j.id})), true);
                j.fail(e);
            }
            j.busy.store(false, Ordering::Release);
        });
        self.status(&id)
    }
    pub fn import(self: &Arc<Self>, id: &str, name: &str) -> Result<Value> {
        let j = self.job(id)?;
        let connection = self.connection(&j.tv)?;
        if j.kind != "playlist"
            || j.status.lock().unwrap()["phase"] != "ready"
            || j.busy.swap(true, Ordering::AcqRel)
        {
            bail!("Verifica prima la lista sulla TV.");
        }
        let name = display(name, 100);
        if name.is_empty() {
            j.busy.store(false, Ordering::Release);
            bail!("Inserisci un nome per la lista sulla TV.");
        }
        j.set(json!({"id":j.id,"phase":"importing","error":""}));
        let id = j.id.clone();
        thread::spawn(move || {
            let result = (|| {
                j.check()?;
                let state = connection.exchange(
                    "/v1/playlists/import",
                    Some(&json!({"id":j.id,"name":name})),
                    true,
                )?;
                wait_playlist(&j, &connection, state)
            })();
            if let Err(e) = result {
                j.fail(e);
            }
            j.busy.store(false, Ordering::Release);
        });
        self.status(&id)
    }
    pub fn cancel(&self, id: &str) -> Result<Value> {
        let j = self.job(id)?;
        j.cancelled.store(true, Ordering::Release);
        if !j.busy.load(Ordering::Acquire) {
            if let Ok(c) = self.connection(&j.tv) {
                if j.kind == "watch" {
                    let _ = c.exchange("/v1/watch/cancel", Some(&json!({"id":id})), true);
                }
                if j.kind == "playlist" {
                    let _ = c.exchange("/v1/playlists/cancel", Some(&json!({"id":id})), true);
                }
            }
            self.jobs.lock().unwrap().remove(id);
        }
        Ok(json!({"cancelled":true}))
    }
}
fn wait_watch(j: &Job, connection: &Connection, mut state: Value, starting: bool) -> Result<()> {
    let until = Instant::now() + Duration::from_secs(60);
    loop {
        j.check()?;
        let status = state["status"].as_str().unwrap_or("");
        if status == "failed" {
            return Err(anyhow!(
                "{}",
                message(state["error"].as_str().unwrap_or("watch_failed"))
            ));
        }
        if !["resolving", "ready", "starting", "started", "cancelled"].contains(&status) {
            bail!("Risposta della TV non valida.");
        }
        j.set(json!({"id":j.id,"phase":status,"title":display(state["title"].as_str().unwrap_or(""),160),"vod":state["vod"],"error":""}));
        if status == "started" || status == "cancelled" || !starting && status == "ready" {
            return Ok(());
        }
        if Instant::now() >= until {
            bail!("La TV non ha confermato l’operazione in tempo. Controlla la TV.");
        }
        thread::sleep(Duration::from_millis(500));
        state = connection.exchange("/v1/watch/status", Some(&json!({"id":j.id})), true)?;
    }
}
fn wait_playlist(j: &Job, connection: &Connection, mut state: Value) -> Result<()> {
    let until = Instant::now() + Duration::from_secs(600);
    loop {
        j.check()?;
        let status = state["status"].as_str().unwrap_or("");
        if status == "failed" {
            bail!("La TV non riesce a importare la lista. Controlla spazio disponibile e versione di Lume sulla TV.");
        }
        if !["preparing", "ready", "importing", "imported", "cancelled"].contains(&status) {
            bail!("Risposta al trasferimento non valida.");
        }
        let mut public = state.clone();
        public["phase"] = json!(status);
        public["id"] = json!(j.id);
        j.set(public);
        if ["ready", "imported", "cancelled"].contains(&status) {
            return Ok(());
        }
        if Instant::now() >= until {
            bail!(
                "La preparazione della lista sulla TV ha richiesto troppo tempo. Controlla la TV."
            );
        }
        thread::sleep(Duration::from_millis(500));
        state = connection.exchange("/v1/playlists/status", Some(&json!({"id":j.id})), true)?;
    }
}
