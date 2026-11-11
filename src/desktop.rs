// SPDX-License-Identifier: GPL-3.0-or-later
//! Persistent desktop application, backed by the tested Rust provider adapter.
use crate::{
    catalog::CatalogService,
    guide::{allowed_logo, GuideService},
    preferences::PreferenceStore,
    providers::Resolver,
    server::HarnessServer,
    updater::UpdateService,
    widevine::WidevineService,
};
use anyhow::{anyhow, Context, Result};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

const MAX_BODY: usize = 2 * 1024 * 1024;

struct Playback {
    audio: Option<Arc<crate::radio_audio::AudioSession>>,
    id: String,
    channel_id: String,
    drm: bool,
    token: String,
    server: HarnessServer,
}
impl Drop for Playback {
    fn drop(&mut self) {
        if let Some(audio) = &self.audio {
            audio.stop();
        }
    }
}

struct DesktopState {
    catalog: Arc<CatalogService>,
    origin: String,
    token: String,
    directory: PathBuf,
    stop: AtomicBool,
    active_requests: AtomicUsize,
    generation: AtomicU64,
    current: Mutex<Option<Arc<Playback>>>,
    preferences: PreferenceStore,
    guide: Arc<GuideService>,
    resolver: Resolver,
    relay: Client,
    artwork: Client,
    widevine: Arc<WidevineService>,
    updater: Arc<UpdateService>,
}

pub struct DesktopServer {
    state: Arc<DesktopState>,
    pub launch_url: String,
    pub origin: String,
    handle: Option<thread::JoinHandle<()>>,
}

impl DesktopServer {
    pub fn updater_service(&self) -> Arc<UpdateService> {
        self.state.updater.clone()
    }
    pub fn widevine_service(&self) -> Arc<WidevineService> {
        self.state.widevine.clone()
    }
    /// A local diagnostic snapshot with no stream URLs, tokens or license data.
    pub fn playback_snapshot(&self) -> Value {
        playback_snapshot(&self.state)
    }

    pub fn start(directory: &Path) -> Result<Self> {
        fs::create_dir_all(directory).context("Impossibile preparare Lume TV")?;
        fs::create_dir_all(directory.join("artwork")).context("Impossibile preparare i loghi")?;
        let server =
            Server::http("127.0.0.1:0").map_err(|_| anyhow!("Impossibile avviare Lume TV"))?;
        let origin = format!(
            "http://{}",
            server
                .server_addr()
                .to_ip()
                .context("Indirizzo locale non disponibile")?
        );
        let token = uuid::Uuid::new_v4().to_string();
        let catalog = CatalogService::new(directory)?;
        let preferences = PreferenceStore::with_catalog(directory, catalog.clone())?;
        let guide = GuideService::with_catalog(directory, catalog.clone())?;
        let relay = Client::builder()
            .no_proxy()
            .referer(false)
            .timeout(Duration::from_secs(45))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| anyhow!("Player non disponibile"))?;
        let artwork = Client::builder()
            .timeout(Duration::from_secs(8))
            .connect_timeout(Duration::from_secs(4))
            .referer(false)
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() < 3 && allowed_logo(attempt.url().as_str()) {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }))
            .build()
            .map_err(|_| anyhow!("Loghi non disponibili"))?;
        let state = Arc::new(DesktopState {
            catalog,
            origin: origin.clone(),
            token: token.clone(),
            directory: directory.to_path_buf(),
            stop: AtomicBool::new(false),
            active_requests: AtomicUsize::new(0),
            generation: AtomicU64::new(0),
            current: Mutex::new(None),
            preferences,
            guide,
            resolver: Resolver::new()?,
            relay,
            artwork,
            widevine: Arc::new(WidevineService::default()),
            updater: UpdateService::new(directory),
        });
        state.catalog.refresh(false);
        state.catalog.radios.refresh(false);
        state.guide.refresh(false);
        let shared = state.clone();
        let handle = thread::spawn(move || {
            while !shared.stop.load(Ordering::Relaxed) {
                if let Ok(Some(request)) = server.recv_timeout(Duration::from_millis(100)) {
                    if shared.active_requests.fetch_add(1, Ordering::Relaxed) >= 32 {
                        shared.active_requests.fetch_sub(1, Ordering::Relaxed);
                        respond_json(request, 503, json!({"error":"busy"}));
                        continue;
                    }
                    let state = shared.clone();
                    thread::spawn(move || {
                        handle_request(request, &state);
                        state.active_requests.fetch_sub(1, Ordering::Relaxed);
                    });
                }
            }
        });
        Ok(Self {
            state,
            launch_url: format!("{origin}/#{token}"),
            origin,
            handle: Some(handle),
        })
    }
}

impl Drop for DesktopServer {
    fn drop(&mut self) {
        self.state.updater.stop();
        self.state.current.lock().expect("playback lock").take();
        self.state.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

pub fn default_data_directory() -> PathBuf {
    if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("LumeTV")
    } else {
        let user_directory = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        if cfg!(target_os = "macos") {
            user_directory.join("Library/Application Support/LumeTV")
        } else {
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| user_directory.join(".local/share"))
                .join("lumetv")
        }
    }
}

fn header<'a>(request: &'a Request, name: &'static str) -> Option<&'a str> {
    request
        .headers()
        .iter()
        .find(|h| h.field.equiv(name))
        .map(|h| h.value.as_str())
}

/// Bundled local UI assets, shared by HTTP preview and the native CEF host.
pub fn asset(path: &str) -> Option<(&'static str, &'static [u8])> {
    match path {
        "/" => Some((
            "text/html; charset=utf-8",
            include_bytes!("../web/app/bundle/index.html"),
        )),
        "/app.css" => Some((
            "text/css; charset=utf-8",
            include_bytes!("../web/app/bundle/app.css"),
        )),
        "/app.js" => Some((
            "text/javascript; charset=utf-8",
            include_bytes!("../web/app/bundle/app.js"),
        )),
        "/player.js" => Some((
            "text/javascript; charset=utf-8",
            include_bytes!("../web/player.js"),
        )),
        "/vendor/shaka-player.compiled.js" => Some((
            "text/javascript; charset=utf-8",
            include_bytes!("../vendor/shaka/dist/shaka-player.compiled.js"),
        )),
        "/assets/logo-light.svg" => Some((
            "image/svg+xml",
            include_bytes!("../web/app/assets/logo-light.svg"),
        )),
        "/assets/logo-dark.svg" => Some((
            "image/svg+xml",
            include_bytes!("../web/app/assets/logo-dark.svg"),
        )),
        "/assets/symbol.svg" => Some((
            "image/svg+xml",
            include_bytes!("../web/app/assets/symbol.svg"),
        )),
        "/favicon.ico" => Some((
            "image/png",
            include_bytes!("../web/app/assets/app-icon.png"),
        )),
        _ => None,
    }
}

fn handle_request(mut request: Request, state: &Arc<DesktopState>) {
    let path = request.url().split('?').next().unwrap_or("").to_owned();
    if header(&request, "Host") != Some(state.origin.trim_start_matches("http://")) {
        respond_json(request, 403, json!({"error":"invalid_host"}));
        return;
    }
    if path.starts_with("/api/")
        && (header(&request, "X-Lume-Token") != Some(state.token.as_str())
            || header(&request, "Origin").is_some_and(|origin| origin != state.origin))
    {
        respond_json(request, 403, json!({"error":"unauthorized"}));
        return;
    }
    if let Some(capability) = path.strip_prefix("/audio/") {
        if header(&request, "Origin").is_some_and(|origin| origin != state.origin) {
            respond_json(request, 403, json!({"error":"unauthorized"}));
            return;
        }
        let audio = state
            .current
            .lock()
            .expect("playback lock")
            .as_ref()
            .and_then(|p| p.audio.clone())
            .filter(|a| a.capability == capability);
        let Some(audio) = audio else {
            respond_json(request, 404, json!({"error":"not_found"}));
            return;
        };
        if request.method() == &Method::Head {
            respond(request, 200, &audio.mime, Vec::new(), vec![]);
            return;
        }
        if request.method() != &Method::Get {
            respond_json(request, 405, json!({"error":"method_not_allowed"}));
            return;
        }
        let Some(reader) = audio.reader() else {
            respond_json(request, 409, json!({"error":"stream_in_use"}));
            return;
        };
        let headers = [
            ("Content-Type", audio.mime.as_str()),
            ("Cache-Control", "no-store"),
            ("Accept-Ranges", "none"),
            ("Referrer-Policy", "no-referrer"),
        ]
        .iter()
        .filter_map(|(k, v)| Header::from_bytes(k.as_bytes(), v.as_bytes()).ok())
        .collect();
        let _ = request.respond(Response::new(StatusCode(200), headers, reader, None, None));
        return;
    }
    if request.method() == &Method::Get {
        let asset = asset(&path);
        if let Some((mime, bytes)) = asset {
            respond(request, 200, mime, bytes.to_vec(), vec![]);
            return;
        }
        match path.as_str() {
            "/api/bootstrap" => {
                let catalog = state.catalog.desktop_snapshot();
                respond_json(
                    request,
                    200,
                    json!({"channels":catalog.channels,"catalog":catalog.status,"preferences":state.preferences.get(),
                    "version":env!("CARGO_PKG_VERSION"),"os":std::env::consts::OS}),
                );
            }
            "/api/catalog" => {
                state.catalog.refresh(false);
                state.catalog.radios.refresh(false);
                respond_json(request, 200, json!(state.catalog.desktop_snapshot()));
            }
            "/api/preferences" => respond_json(request, 200, json!(state.preferences.get())),
            "/api/guide" => {
                state.guide.refresh(false);
                respond_json(request, 200, json!(state.guide.snapshot()));
            }
            "/api/playback/status" => {
                respond_json(request, 200, playback_snapshot(state));
            }
            "/api/radio/status" => {
                let current = state.current.lock().expect("playback lock").clone();
                let value = current
                    .map(|p| {
                        json!({
                            "playback_id": p.id,
                            "channel_id": p.channel_id,
                            "audio": p.audio.as_ref().map(|a| a.snapshot())
                        })
                    })
                    .unwrap_or_else(|| json!({}));
                respond_json(request, 200, value);
            }
            "/api/widevine" => respond_json(request, 200, json!(state.widevine.snapshot())),
            "/api/updates" => respond_json(request, 200, json!(state.updater.snapshot())),
            _ if path.starts_with("/art/channel/") => channel_art(request, state, &path[13..]),
            _ => respond_json(request, 404, json!({"error":"not_found"})),
        }
        return;
    }
    if request.method() != &Method::Post {
        respond_json(request, 405, json!({"error":"method_not_allowed"}));
        return;
    }
    let mut body = vec![];
    if request
        .as_reader()
        .take(MAX_BODY as u64 + 1)
        .read_to_end(&mut body)
        .is_err()
        || body.len() > MAX_BODY
    {
        respond_json(request, 413, json!({"error":"request_too_large"}));
        return;
    }
    let Ok(value) = serde_json::from_slice::<Value>(&body) else {
        respond_json(request, 400, json!({"error":"invalid_json"}));
        return;
    };
    match path.as_str() {
        "/api/updates/check" => respond_json(
            request,
            200,
            json!(state
                .updater
                .check(value.get("manual").and_then(Value::as_bool).unwrap_or(true))),
        ),
        "/api/updates/cancel" => respond_json(request, 200, json!(state.updater.cancel())),
        "/api/updates/download" | "/api/updates/install" | "/api/updates/settings" => {
            let result = match path.as_str() {
                "/api/updates/download" => state.updater.download(),
                "/api/updates/install" => state.updater.install(),
                _ => state.updater.settings(
                    value.get("automatic").and_then(Value::as_bool),
                    value.get("preview").and_then(Value::as_bool),
                ),
            };
            match result {
                Ok(snapshot) => respond_json(request, 200, json!(snapshot)),
                Err(error) => respond_json(request, 409, json!({"error":error})),
            }
        }
        "/api/widevine/update" => {
            respond_json(request, 200, json!(state.widevine.request_update()))
        }
        "/api/catalog/refresh" => {
            state.catalog.refresh(true);
            respond_json(request, 200, json!(state.catalog.desktop_snapshot()));
        }
        "/api/radios/refresh" => {
            state.catalog.radios.refresh(true);
            respond_json(request, 200, json!(state.catalog.desktop_snapshot()));
        }
        "/api/preferences" => match state.preferences.update(&value) {
            Ok(preferences) => respond_json(request, 200, json!(preferences)),
            Err(_) => respond_json(request, 400, json!({"error":"preferences_invalid"})),
        },
        "/api/guide/refresh" => {
            state.guide.refresh(true);
            respond_json(request, 200, json!(state.guide.snapshot()));
        }
        "/api/play" => play(request, state, &value),
        "/api/stop" => {
            let mut current = state.current.lock().expect("playback lock");
            if value["playback_id"]
                .as_str()
                .is_some_and(|id| current.as_ref().is_none_or(|p| p.id != id))
            {
                drop(current);
                respond_json(request, 409, json!({"error":"playback_changed"}));
                return;
            }
            state.generation.fetch_add(1, Ordering::SeqCst);
            let old = current.take();
            drop(current);
            drop(old);
            respond_json(request, 200, json!({"stopped":true}));
        }
        "/api/played" => {
            let current = state.current.lock().expect("playback lock").clone();
            if let Some(current) = current.filter(|p| value["playback_id"] == p.id) {
                match state
                    .preferences
                    .update(&json!({"played":current.channel_id}))
                {
                    Ok(preferences) => respond_json(request, 200, json!(preferences)),
                    Err(_) => respond_json(request, 500, json!({"error":"preferences_write"})),
                }
            } else {
                respond_json(request, 409, json!({"error":"playback_changed"}));
            }
        }
        "/api/fetch" | "/api/event" => relay_playback(request, state, &path, body),
        _ => respond_json(request, 404, json!({"error":"not_found"})),
    }
}

fn playback_snapshot(state: &DesktopState) -> Value {
    let current = state.current.lock().expect("playback lock").clone();
    let Some(current) = current else {
        return json!({"channel_id":null,"events":[]});
    };
    let events = current
        .server
        .state
        .events
        .lock()
        .map(|events| events.clone())
        .unwrap_or_default();
    let network = current.server.network_summary();
    let assessment = crate::report::assess_playback(
        &events,
        30,
        current.drm,
        network["license_http_successes"].as_u64().unwrap_or(0) > 0,
    );
    json!({
        "channel_id": current.channel_id,
        "events": events,
        "network": network,
        "assessment": assessment,
        "audio": current.audio.as_ref().map(|audio| audio.snapshot())
    })
}

fn play(request: Request, state: &DesktopState, value: &Value) {
    let Some(channel) = value["channel_id"]
        .as_str()
        .and_then(|id| state.catalog.channel(id))
    else {
        respond_json(request, 400, json!({"error":"channel_unknown"}));
        return;
    };
    let generation = state.generation.fetch_add(1, Ordering::SeqCst) + 1;
    let mut stream = match state.resolver.resolve(&channel.stream_channel()) {
        Ok(stream) => stream,
        Err(_) => {
            respond_json(request, 502, json!({"error":"channel_unavailable"}));
            return;
        }
    };
    let audio = if channel.kind == crate::model::MediaKind::Radio && stream.mime_type.is_none() {
        match crate::radio_audio::AudioSession::connect(&stream.url) {
            Ok(crate::radio_audio::Connection::Audio(audio)) => {
                stream.mime_type = Some(audio.mime.clone());
                Some(audio)
            }
            Ok(crate::radio_audio::Connection::Manifest(mime)) => {
                stream.mime_type = Some(mime.into());
                None
            }
            Err(_) => {
                respond_json(request, 502, json!({"error":"radio_unavailable"}));
                return;
            }
        }
    } else {
        None
    };
    let drm = stream.drm.is_some();
    let uri = audio
        .as_ref()
        .map(|a| format!("{}/audio/{}", state.origin, a.capability))
        .unwrap_or_else(|| stream.url.clone());
    let session = json!({"url":uri, "mime_type":stream.mime_type,"continuous":audio.is_some(),
        "drm":stream.drm.as_ref().map(|_| json!({"key_system":"com.widevine.alpha"}))});
    let server = match HarnessServer::start(channel.stream_channel(), stream, 0, "relay") {
        Ok(server) => server,
        Err(_) => {
            respond_json(request, 500, json!({"error":"player_unavailable"}));
            return;
        }
    };
    let token = server
        .launch_url
        .split_once('#')
        .expect("session token")
        .1
        .to_owned();
    let playback = Arc::new(Playback {
        audio,
        id: uuid::Uuid::new_v4().to_string(),
        channel_id: channel.id.clone(),
        drm,
        token,
        server,
    });
    let mut guard = state.current.lock().expect("playback lock");
    if state.generation.load(Ordering::SeqCst) != generation {
        drop(guard);
        respond_json(request, 409, json!({"error":"playback_changed"}));
        return;
    }
    let previous = guard.replace(playback.clone());
    drop(guard);
    drop(previous);
    respond_json(
        request,
        200,
        json!({"playback_id":playback.id, "channel":channel, "stream":session}),
    );
}

fn relay_playback(request: Request, state: &DesktopState, path: &str, body: Vec<u8>) {
    let playback_id = header(&request, "X-Lume-Playback-Id")
        .unwrap_or_default()
        .to_owned();
    let current = state.current.lock().expect("playback lock").clone();
    let Some(current) = current.filter(|p| p.id == playback_id) else {
        respond_json(request, 409, json!({"error":"playback_changed"}));
        return;
    };
    let result = (|| -> Result<_> {
        let response = state
            .relay
            .post(format!("{}{path}", current.server.origin))
            .header("X-Lume-Token", &current.token)
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .map_err(|_| anyhow!("player_adapter"))?;
        let status = response.status().as_u16();
        let mime = response
            .headers()
            .get("Content-Type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_owned();
        let mut extra = vec![];
        for name in ["X-Lume-Final-Url", "Content-Range", "Accept-Ranges"] {
            if let Some(value) = response.headers().get(name).and_then(|v| v.to_str().ok()) {
                extra.push((name.to_owned(), value.to_owned()));
            }
        }
        let mut bytes = vec![];
        response
            .take(32 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| anyhow!("player_adapter_read"))?;
        if bytes.len() > 32 * 1024 * 1024 {
            return Err(anyhow!("player_adapter_limit"));
        }
        Ok((status, mime, bytes, extra))
    })();
    match result {
        Ok((status, mime, bytes, extra)) => respond(request, status, &mime, bytes, extra),
        Err(_) => respond_json(request, 502, json!({"error":"playback_network"})),
    }
}

fn channel_art(request: Request, state: &DesktopState, id: &str) {
    if state.catalog.channel(id).is_none() {
        respond_json(request, 404, json!({"error":"channel_unknown"}));
        return;
    }
    let cache = state.directory.join("artwork").join(format!("{id}.png"));
    if let Ok(bytes) = fs::read(&cache) {
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() <= 1024 * 1024 {
            respond(request, 200, "image/png", bytes, vec![]);
            return;
        }
    }
    let result = (|| -> Result<Vec<u8>> {
        let url = state.guide.logo_url(id).context("no_artwork")?;
        let response = state
            .artwork
            .get(url)
            .send()
            .map_err(|_| anyhow!("artwork_network"))?;
        if !response.status().is_success() {
            return Err(anyhow!("artwork_response"));
        }
        let mut bytes = vec![];
        response
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| anyhow!("artwork_read"))?;
        if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes.len() > 1024 * 1024 {
            return Err(anyhow!("artwork_type"));
        }
        let _ = fs::write(cache, &bytes);
        Ok(bytes)
    })();
    match result {
        Ok(bytes) => respond(request, 200, "image/png", bytes, vec![]),
        Err(_) => respond_json(request, 404, json!({"error":"artwork_unavailable"})),
    }
}

fn respond_json(request: Request, status: u16, value: Value) {
    respond(
        request,
        status,
        "application/json; charset=utf-8",
        value.to_string().into_bytes(),
        vec![],
    );
}
fn respond(
    request: Request,
    status: u16,
    mime: &str,
    bytes: Vec<u8>,
    extra: Vec<(String, String)>,
) {
    let mut response = Response::from_data(bytes).with_status_code(StatusCode(status));
    for (name, value) in [("Content-Type",mime),("Cache-Control","no-store"),("Referrer-Policy","no-referrer"),
        ("X-Content-Type-Options","nosniff"),("Content-Security-Policy","default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self' https:; media-src 'self' blob: https:; worker-src 'self' blob:; object-src 'none'; frame-ancestors 'none'")] {
        if let Ok(header) = Header::from_bytes(name, value) { response.add_header(header); }
    }
    for (name, value) in extra {
        if let Ok(header) = Header::from_bytes(name, value) {
            response.add_header(header);
        }
    }
    let _ = request.respond(response);
}
