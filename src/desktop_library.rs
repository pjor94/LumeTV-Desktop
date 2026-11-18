// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::personal::{CatalogQuery, ImportConfig, Resource};

pub(super) fn respond_result(request: Request, result: Result<Value>) {
    match result {
        Ok(value) => respond_json(request, 200, value),
        Err(error) => respond_json(request, 400, json!({"error":error.to_string()})),
    }
}
pub(super) fn handle(request: Request, state: &Arc<DesktopState>, path: &str, value: &Value) {
    let library = &state.catalog.personal;
    let id = value["id"].as_str().unwrap_or("");
    let result = (|| -> Result<Value> {
        Ok(match path {
            "/api/library/backup/input" => state.backups.input()?,
            "/api/library/backup/export" => {
                let mut builtins = state.catalog.snapshot().channels;
                builtins.extend(state.catalog.radios.snapshot().0);
                state.backups.export(
                    library.clone(),
                    state.preferences.get(),
                    builtins,
                    value["password"].as_str().unwrap_or("").into(),
                )?
            }
            "/api/library/backup/prepare" => state
                .backups
                .prepare(id, value["password"].as_str().unwrap_or("").into())?,
            "/api/library/backup/status" => state.backups.status(id)?,
            "/api/library/backup/cancel" => state.backups.cancel(id)?,
            "/api/library/backup/restore" => {
                let mut builtins = state.catalog.snapshot().channels;
                builtins.extend(state.catalog.radios.snapshot().0);
                state
                    .backups
                    .restore(id, value, library, &state.preferences, &builtins)?
            }
            "/api/library/prepare" => library.prepare(
                serde_json::from_value::<ImportConfig>(value.clone())
                    .map_err(|_| anyhow!("Configurazione dell’importazione non valida."))?,
                false,
            )?,
            "/api/library/job" => library.job(id)?,
            "/api/library/commit" => library.commit(id)?,
            "/api/library/cancel" => library.cancel(id)?,
            "/api/library/source" => library.detail(id)?,
            "/api/library/edit" => library.edit(
                id,
                value["revision"].as_u64().unwrap_or(0),
                value["name"].as_str().unwrap_or(""),
                value["enabled"].as_bool().unwrap_or(false),
                serde_json::from_value(value["settings"].clone())
                    .map_err(|_| anyhow!("Impostazioni della lista non valide."))?,
            )?,
            "/api/library/delete" => library.delete(id, value["revision"].as_u64().unwrap_or(0))?,
            "/api/library/refresh" => library.refresh(id, true)?,
            "/api/library/home" => library.home(
                value["source"].as_str().unwrap_or(""),
                &state.preferences.get().recent,
            )?,
            "/api/library/video/page" => library.videos(
                &serde_json::from_value::<crate::personal::video::Query>(value.clone())
                    .map_err(|_| anyhow!("Filtri video non validi."))?,
                &state.preferences.get().favorites,
            )?,
            "/api/library/video/home" => library.video_home(
                value["source"].as_str().unwrap_or(""),
                &state.preferences.get().favorites,
            )?,
            "/api/library/video/detail" => {
                library.video_detail(id, &state.preferences.get().favorites)?
            }
            "/api/library/video/action" => {
                let enabled = value["enabled"].as_bool().unwrap_or(false);
                let mut result =
                    library.video_action(id, value["action"].as_str().unwrap_or(""), enabled)?;
                if value["action"] == "saved"
                    && !enabled
                    && state
                        .preferences
                        .get()
                        .favorites
                        .iter()
                        .any(|saved| saved == id)
                {
                    state.preferences.update(&json!({"toggle_favorite":id}))?;
                }
                result["preferences"] = json!(state.preferences.get());
                result
            }
            "/api/library/video/settings" => library.video_online_settings(value)?,
            "/api/library/video/metadata" => {
                library.video_metadata(id, value["query"].as_str(), None)?
            }
            "/api/library/video/choose" => library.video_metadata(
                id,
                None,
                Some(
                    value["candidate"]
                        .as_u64()
                        .and_then(|n| u32::try_from(n).ok())
                        .filter(|n| *n > 0),
                ),
            )?,
            "/api/library/catalog" | "/api/library/groups" => {
                let query: CatalogQuery = serde_json::from_value(value.clone())
                    .map_err(|_| anyhow!("Filtri del catalogo non validi."))?;
                if path.ends_with("groups") {
                    library.groups(&query)?
                } else {
                    library.catalog(&query, &state.preferences.get().favorites)?
                }
            }
            "/api/library/channel" => serde_json::to_value(
                library
                    .channel(id)
                    .context("Il canale non è più disponibile.")?,
            )?,
            "/api/library/next" => {
                json!({"channel":library.next(id,value["direction"].as_i64().unwrap_or(1)as i32,&state.preferences.get().favorites,state.preferences.get().zap_favorites_only)?})
            }
            "/api/library/guide/refresh" => {
                library.refresh_guide(id, true);
                library.snapshot()
            }
            "/api/library/guide/clear" => library.clear_guide(id)?,
            "/api/library/guide/choices" => library.guide_choices(
                id,
                value["query"].as_str().unwrap_or(""),
                value["offset"].as_u64().unwrap_or(0).min(usize::MAX as u64) as usize,
            )?,
            "/api/library/guide/window" => library.guide_window(
                id,
                value["from"].as_u64().unwrap_or_else(crate::report::now_ms),
                value["until"]
                    .as_u64()
                    .unwrap_or_else(|| crate::report::now_ms() + 86_400_000),
                value["query"].as_str().unwrap_or(""),
                value["offset"].as_u64().unwrap_or(0).min(usize::MAX as u64) as usize,
            )?,
            "/api/library/watch" => {
                library.watch_checked(
                    id,
                    value["mode"].as_str().unwrap_or(""),
                    value["event"].as_str().unwrap_or(""),
                    value["position"].as_f64().unwrap_or(0.0),
                    value["duration"].as_f64().unwrap_or(0.0),
                    value["completed"].as_bool().unwrap_or(false),
                    value["revision"].as_u64(),
                )?;
                json!({"saved":true})
            }
            _ => return Err(anyhow!("Azione della libreria non disponibile.")),
        })
    })();
    respond_result(request, result);
}

struct StreamReader {
    response: crate::personal::network::NetworkResponse,
    context: Arc<crate::personal::Context>,
    cancel: Arc<AtomicBool>,
}
impl Read for StreamReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::Acquire) || self.context.check().is_err() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "playback_cancelled",
            ));
        }
        self.response.read(buffer)
    }
}
pub(super) fn stream(request: Request, state: &DesktopState, capability: &str) {
    if header(&request, "Origin").is_some_and(|origin| origin != state.origin) {
        respond_json(request, 403, json!({"error":"unauthorized"}));
        return;
    }
    if request.method() != &Method::Get && request.method() != &Method::Head {
        respond_json(request, 405, json!({"error":"method_not_allowed"}));
        return;
    }
    let current = state.current.lock().expect("playback").clone();
    let Some(current) = current.filter(|p| p.id == capability) else {
        respond_json(request, 404, json!({"error":"stream_unavailable"}));
        return;
    };
    let Some(policy) = current.server.state.personal.clone() else {
        respond_json(request, 404, json!({"error":"stream_unavailable"}));
        return;
    };
    let cancel = current.cancel.clone();
    let root = current.server.state.stream_url();
    drop(current);
    let range = header(&request, "Range").map(str::to_owned);
    let result = policy.context.range_request(
        &policy.root,
        &root,
        Resource::Media,
        &policy.media,
        range.as_deref(),
    );
    let response = match result {
        Ok(response) if response.status().is_success() => response,
        _ => {
            respond_json(request, 502, json!({"error":"stream_unavailable"}));
            return;
        }
    };
    let status = StatusCode(response.status().as_u16());
    let length = response
        .content_length()
        .and_then(|n| usize::try_from(n).ok());
    let mut headers = vec![];
    for name in ["Content-Type", "Content-Range", "Accept-Ranges"] {
        if let Some(value) = response.headers().get(name).and_then(|v| v.to_str().ok()) {
            if let Ok(header) = Header::from_bytes(name, value) {
                headers.push(header);
            }
        }
    }
    for (name, value) in [
        ("Cache-Control", "no-store"),
        ("Referrer-Policy", "no-referrer"),
        ("X-Content-Type-Options", "nosniff"),
        ("Content-Security-Policy", "default-src 'none'; sandbox"),
    ] {
        headers.push(Header::from_bytes(name, value).unwrap());
    }
    if request.method() == &Method::Head {
        let _ = request.respond(Response::new(
            status,
            headers,
            std::io::empty(),
            length,
            None,
        ));
        return;
    }
    let _ = request.respond(Response::new(
        status,
        headers,
        StreamReader {
            response,
            context: policy.context.clone(),
            cancel,
        },
        length,
        None,
    ));
}

pub(super) fn artwork(request: Request, state: &DesktopState, id: &str) {
    let result = state.catalog.personal.logo(id);
    match result {
        Ok((mime, bytes)) => respond(request, 200, &mime, bytes, vec![]),
        Err(_) => match state.catalog.personal.public_logo_candidates(id) {
            Ok(candidates) if !candidates.is_empty() => {
                super::public_channel_art(request, state, candidates)
            }
            _ => respond_json(request, 404, json!({"error":"artwork_unavailable"})),
        },
    }
}
