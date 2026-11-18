// SPDX-License-Identifier: GPL-3.0-or-later
//! Persistent desktop application, backed by the tested Rust provider adapter.
use crate::{
    catalog::CatalogService,
    guide::{allowed_logo_redirect, GuideService},
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
#[path = "desktop_companion.rs"]
mod companion_api;
#[path = "desktop_library.rs"]
mod personal_api;

const MAX_BODY: usize = 2 * 1024 * 1024;

struct Playback {
    mode: String,
    cancel: Arc<AtomicBool>,
    audio: Option<Arc<crate::radio_audio::AudioSession>>,
    id: String,
    channel_id: String,
    drm: bool,
    token: String,
    server: HarnessServer,
}
impl Playback {
    fn stop(&self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(audio) = &self.audio {
            audio.stop();
        }
    }
}
impl Drop for Playback {
    fn drop(&mut self) {
        self.stop();
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
    backups: Arc<crate::personal::backup::Backups>,
    companion: Arc<crate::companion::Companion>,
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
            .user_agent(crate::providers::USER_AGENT)
            .no_proxy()
            .dns_resolver(Arc::new(crate::network::PublicDns))
            .timeout(Duration::from_secs(8))
            .connect_timeout(Duration::from_secs(4))
            .referer(false)
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() < 3
                    && attempt
                        .previous()
                        .last()
                        .is_some_and(|from| allowed_logo_redirect(from, attempt.url()))
                {
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
            backups: crate::personal::backup::Backups::new(directory)?,
            companion: crate::companion::Companion::new(directory)?,
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
        state.catalog.personal.refresh_due();
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
        "/assets/tmdb.svg" => Some((
            "image/svg+xml",
            include_bytes!("../web/app/assets/tmdb.svg"),
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
    if let Some(id) = path
        .strip_prefix("/backup/")
        .and_then(|s| s.strip_suffix("/Lume.lumebackup"))
    {
        if request.method() != &Method::Get
            || header(&request, "Origin").is_some_and(|o| o != state.origin)
        {
            respond_json(request, 403, json!({"error":"unauthorized"}));
            return;
        }
        match state.backups.download(id) {
            Ok(file) => {
                let size = file.metadata().ok().map(|m| m.len() as usize);
                let headers = [
                    ("Content-Type", "application/octet-stream"),
                    (
                        "Content-Disposition",
                        r#"attachment; filename="Lume.lumebackup""#,
                    ),
                    ("Cache-Control", "no-store"),
                    ("Referrer-Policy", "no-referrer"),
                ]
                .iter()
                .filter_map(|(k, v)| Header::from_bytes(k.as_bytes(), v.as_bytes()).ok())
                .collect();
                let _ = request.respond(Response::new(StatusCode(200), headers, file, size, None));
            }
            Err(_) => respond_json(request, 404, json!({"error":"not_found"})),
        }
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
    if let Some(capability) = path.strip_prefix("/stream/") {
        personal_api::stream(request, state, capability);
        return;
    }
    if request.method() == &Method::Get {
        let asset = asset(&path);
        if let Some((mime, bytes)) = asset {
            respond(request, 200, mime, bytes.to_vec(), vec![]);
            return;
        }
        match path.as_str() {
            "/api/library" => {
                state.catalog.personal.refresh_due();
                let mut snapshot = state.catalog.personal.snapshot();
                snapshot["favorite_count"] = state
                    .catalog
                    .personal
                    .catalog(
                        &crate::personal::CatalogQuery {
                            kind: "tv".into(),
                            content: "live".into(),
                            favorites: true,
                            limit: 1,
                            ..Default::default()
                        },
                        &state.preferences.get().favorites,
                    )
                    .ok()
                    .and_then(|v| v.get("total").cloned())
                    .unwrap_or(json!(0));
                respond_json(request, 200, snapshot);
            }
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
            _ if path.starts_with("/art/video/") => {
                let parts = path[11..].split('/').collect::<Vec<_>>();
                if parts.len() == 2 && ["poster", "backdrop"].contains(&parts[1]) {
                    match state
                        .catalog
                        .personal
                        .video_artwork(parts[0], parts[1] == "backdrop")
                    {
                        Ok((mime, bytes)) => respond(request, 200, &mime, bytes, vec![]),
                        Err(_) => {
                            respond_json(request, 404, json!({"error":"artwork_unavailable"}))
                        }
                    }
                } else {
                    respond_json(request, 404, json!({"error":"not_found"}));
                }
            }
            _ => respond_json(request, 404, json!({"error":"not_found"})),
        }
        return;
    }
    if request.method() != &Method::Post {
        respond_json(request, 405, json!({"error":"method_not_allowed"}));
        return;
    }
    if let Some(id) = path.strip_prefix("/api/library/backup/upload/") {
        let result = state.backups.upload(id, request.as_reader());
        personal_api::respond_result(request, result);
        return;
    }
    if let Some(id) = path.strip_prefix("/api/library/upload/") {
        let result = state.catalog.personal.upload(id, request.as_reader());
        personal_api::respond_result(request, result);
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
    if path.starts_with("/api/companion/") {
        companion_api::handle(request, state, &path, &value);
        return;
    }
    if path.starts_with("/api/library/") {
        personal_api::handle(request, state, &path, &value);
        return;
    }
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
            if let Some(old) = &old {
                old.stop();
            }
            drop(current);
            drop(old);
            respond_json(request, 200, json!({"stopped":true}));
        }
        "/api/played" => {
            let current = state.current.lock().expect("playback lock").clone();
            if let Some(current) = current.filter(|p| value["playback_id"] == p.id) {
                if current.mode != "live" {
                    respond_json(request, 200, json!(state.preferences.get()));
                    return;
                }
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
    let local_clearkey = current
        .server
        .state
        .personal
        .as_ref()
        .and_then(|p| p.drm.as_ref())
        .is_some_and(|d| d.system == "org.w3.clearkey" && d.local.is_some());
    let license = if local_clearkey {
        crate::report::LicenseEvidence::LocalClearKey
    } else if network["license_http_successes"].as_u64().unwrap_or(0) > 0 {
        crate::report::LicenseEvidence::Http
    } else {
        crate::report::LicenseEvidence::None
    };
    let assessment = crate::report::assess_playback_with_license(&events, 30, current.drm, license);
    json!({
        "channel_id": current.channel_id,
        "events": events,
        "network": network,
        "assessment": assessment,
        "audio": current.audio.as_ref().map(|audio| audio.snapshot())
    })
}

fn play(request: Request, state: &DesktopState, value: &Value) {
    let id = value["channel_id"].as_str().unwrap_or("");
    if state.catalog.personal.is_hidden(id) {
        respond_json(
            request,
            400,
            json!({"error":"channel_hidden","message":"Il canale è nascosto. Ripristinalo dalle impostazioni."}),
        );
        return;
    }
    let generation = state.generation.fetch_add(1, Ordering::SeqCst) + 1;
    let personal = if id.starts_with("personal_") {
        match state.catalog.personal.playback(
            id,
            value["mode"].as_str().unwrap_or(""),
            value["event"].as_str().unwrap_or(""),
            value["position"].as_f64(),
        ) {
            Ok(playback) => Some(playback),
            Err(error) => {
                respond_json(
                    request,
                    400,
                    json!({"error":"personal_unavailable","message":error.to_string()}),
                );
                return;
            }
        }
    } else {
        None
    };
    let Some(channel) = personal
        .as_ref()
        .map(|p| p.channel.channel.clone())
        .or_else(|| state.catalog.channel(id))
    else {
        respond_json(request, 400, json!({"error":"channel_unknown"}));
        return;
    };
    let mut stream = match personal
        .as_ref()
        .map(|p| Ok(p.stream.clone()))
        .unwrap_or_else(|| state.resolver.resolve(&channel.stream_channel()))
    {
        Ok(stream) => stream,
        Err(_) => {
            respond_json(request, 502, json!({"error":"channel_unavailable"}));
            return;
        }
    };
    let audio = if personal.is_none()
        && channel.kind == crate::model::MediaKind::Radio
        && stream.mime_type.is_none()
    {
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
    let playback_id = uuid::Uuid::new_v4().to_string();
    let progressive = personal.is_some()
        && !drm
        && stream
            .mime_type
            .as_deref()
            .is_some_and(|m| m.starts_with("video/") || m.starts_with("audio/"));
    let uri = if progressive {
        format!("{}/stream/{playback_id}", state.origin)
    } else {
        audio
            .as_ref()
            .map(|a| format!("{}/audio/{}", state.origin, a.capability))
            .unwrap_or_else(|| stream.url.clone())
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let policy = personal
        .as_ref()
        .map(|p| p.policy.for_playback(cancel.clone()));
    let key_system = policy
        .as_ref()
        .and_then(|p| p.key_system())
        .unwrap_or("com.widevine.alpha");
    let clear_keys = match policy
        .as_ref()
        .and_then(|p| p.drm.as_ref())
        .map(|d| d.local_clear_keys())
        .transpose()
    {
        Ok(keys) => keys.flatten(),
        Err(_) => {
            respond_json(request, 400, json!({"error":"personal_drm_invalid"}));
            return;
        }
    };
    let session = json!({"url":uri,"mime_type":stream.mime_type,"continuous":audio.is_some()||progressive,
        "drm":if drm{Some(json!({"key_system":key_system,"clear_keys":clear_keys}))}else{None},
        "mode":personal.as_ref().map(|p|p.mode.as_str()).unwrap_or("live"),
        "title":personal.as_ref().map(|p|p.title.as_str()).unwrap_or(&channel.name),
        "position":personal.as_ref().map(|p|p.position).unwrap_or(0.0),
        "duration":personal.as_ref().map(|p|p.duration).unwrap_or(0.0),
        "base_offset":personal.as_ref().map(|p|p.base_offset).unwrap_or(0.0),
        "event":personal.as_ref().map(|p|p.event.as_str()).unwrap_or(""),
        "watch_revision":personal.as_ref().map(|p|p.watch_revision),
        "seekable":personal.as_ref().is_none_or(|p|p.seekable)});
    let server = match HarnessServer::start_with_policy(
        channel.stream_channel(),
        stream,
        0,
        "relay",
        policy,
    ) {
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
        mode: personal
            .as_ref()
            .map(|p| p.mode.clone())
            .unwrap_or_else(|| "live".into()),
        cancel,
        audio,
        id: playback_id,
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
    let old = guard.replace(playback.clone());
    if let Some(old) = &old {
        old.stop();
    }
    drop(guard);
    drop(old);
    let display = personal
        .as_ref()
        .map(|p| json!(p.channel))
        .unwrap_or_else(|| json!(channel));
    respond_json(
        request,
        200,
        json!({"playback_id":playback.id,"channel":display,"stream":session}),
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
    if id.starts_with("personal_") {
        personal_api::artwork(request, state, id);
        return;
    }
    if state.catalog.channel(id).is_none() {
        respond_json(request, 404, json!({"error":"channel_unknown"}));
        return;
    }
    let cache = state.directory.join("artwork").join(format!("{id}.png"));
    if let Ok(bytes) = fs::read(&cache) {
        if let Some(mime) = artwork_mime(&bytes) {
            respond(request, 200, mime, bytes, vec![]);
            return;
        }
    }
    if let Ok(bytes) = fetch_artwork(&state.artwork, state.guide.logo_candidates(id)) {
        let _ = fs::write(cache, &bytes);
        respond(request, 200, artwork_mime(&bytes).unwrap(), bytes, vec![]);
        return;
    }
    respond_json(request, 404, json!({"error":"artwork_unavailable"}));
}

fn public_channel_art(request: Request, state: &DesktopState, candidates: Vec<String>) {
    use sha2::Digest;
    let directory = state.directory.join("artwork/public");
    let key = format!("{:x}.img", sha2::Sha256::digest(candidates.join("\n")));
    let cache = directory.join(&key);
    if let Ok(bytes) = fs::read(&cache) {
        if let Some(mime) = artwork_mime(&bytes) {
            respond(request, 200, mime, bytes, vec![]);
            return;
        }
    }
    if let Ok(bytes) = fetch_artwork(&state.artwork, candidates) {
        if fs::create_dir_all(&directory).is_ok() && fs::write(&cache, &bytes).is_ok() {
            let mut files = fs::read_dir(&directory)
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let stem = name.strip_suffix(".img")?;
                    if stem.len() != 64 || !stem.bytes().all(|b| b.is_ascii_hexdigit()) {
                        return None;
                    }
                    let metadata = entry.metadata().ok()?;
                    Some((entry.path(), metadata.len(), metadata.modified().ok()))
                })
                .collect::<Vec<_>>();
            files.sort_by_key(|(_, _, time)| *time);
            let mut total = files.iter().map(|(_, length, _)| *length).sum::<u64>();
            for (path, length, _) in files {
                if total <= 64 * 1024 * 1024 {
                    break;
                }
                if path != cache && fs::remove_file(path).is_ok() {
                    total = total.saturating_sub(length);
                }
            }
        }
        respond(request, 200, artwork_mime(&bytes).unwrap(), bytes, vec![]);
        return;
    }
    respond_json(request, 404, json!({"error":"artwork_unavailable"}));
}

fn fetch_artwork(client: &Client, candidates: Vec<String>) -> Result<Vec<u8>> {
    for url in candidates {
        let result = (|| -> Result<Vec<u8>> {
            let response = client
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
            if artwork_mime(&bytes).is_none() {
                return Err(anyhow!("artwork_type"));
            }
            Ok(bytes)
        })();
        if let Ok(bytes) = result {
            return Ok(bytes);
        }
    }
    Err(anyhow!("artwork_unavailable"))
}

fn artwork_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() > 1024 * 1024 {
        None
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else {
        None
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

#[cfg(test)]
mod artwork_tests {
    use super::*;

    #[test]
    fn unavailable_catalogue_logo_falls_back_to_guide_with_android_identity() {
        use base64::Engine;
        let png = base64::engine::general_purpose::STANDARD
            .decode(include_str!("../tests/fixtures/guide-logo.png.base64").trim())
            .unwrap();
        let expected = png.clone();
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        let worker = std::thread::spawn(move || {
            for (path, status) in [("/catalogue", 404), ("/guide", 200)] {
                let request = server
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap();
                assert_eq!(request.url(), path);
                assert_eq!(
                    request
                        .headers()
                        .iter()
                        .find(|h| h.field.equiv("User-Agent"))
                        .unwrap()
                        .value
                        .as_str(),
                    crate::providers::USER_AGENT
                );
                request
                    .respond(tiny_http::Response::from_data(png.clone()).with_status_code(status))
                    .unwrap();
            }
        });
        let client = Client::builder()
            .no_proxy()
            .user_agent(crate::providers::USER_AGENT)
            .build()
            .unwrap();
        assert_eq!(
            fetch_artwork(
                &client,
                vec![
                    format!("http://{address}/catalogue"),
                    format!("http://{address}/guide")
                ]
            )
            .unwrap(),
            expected
        );
        worker.join().unwrap();
    }

    #[test]
    fn catalogue_image_types_are_sniffed_and_documents_or_oversized_files_rejected() {
        for (bytes, mime) in [
            (b"\x89PNG\r\n\x1a\n".as_slice(), "image/png"),
            (b"\xff\xd8\xff".as_slice(), "image/jpeg"),
            (b"GIF87a".as_slice(), "image/gif"),
            (b"GIF89a".as_slice(), "image/gif"),
            (b"RIFF\x04\x00\x00\x00WEBP".as_slice(), "image/webp"),
        ] {
            assert_eq!(artwork_mime(bytes), Some(mime));
        }
        for bytes in [
            b"<html>unavailable</html>".as_slice(),
            b"<svg onload='x'/>",
            b"RIFF",
        ] {
            assert_eq!(artwork_mime(bytes), None);
        }
        let mut huge = vec![0; 1024 * 1024 + 1];
        huge[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        assert_eq!(artwork_mime(&huge), None);
    }
}
