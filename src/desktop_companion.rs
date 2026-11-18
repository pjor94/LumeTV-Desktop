// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
fn reference(state: &DesktopState, id: &str) -> Result<Value> {
    if state.catalog.personal.is_hidden(id) {
        anyhow::bail!("Il canale è nascosto. Ripristinalo dalle impostazioni.");
    }
    if id.starts_with("personal_") {
        return state.catalog.personal.companion_reference(id);
    }
    let channel = state
        .catalog
        .channel(id)
        .ok_or_else(|| anyhow!("Contenuto non disponibile."))?;
    Ok(
        json!({"reference":{"builtin":crate::personal::wire::builtin_id(&channel),"source":"","lookup":"","entry":"","vod":false,"kind":if channel.kind==crate::model::MediaKind::Radio{"RADIO"}else{"TV"}}}),
    )
}
pub(super) fn handle(request: Request, state: &Arc<DesktopState>, path: &str, value: &Value) {
    let id = value["id"].as_str().unwrap_or("");
    let tv = value["tv"].as_str().unwrap_or("");
    let result = (|| -> Result<Value> {
        Ok(match path {
            "/api/companion/devices" => state.companion.devices(),
            "/api/companion/discover" => state.companion.discover()?,
            "/api/companion/pair" => state.companion.pair(value)?,
            "/api/companion/status" => state.companion.status(id)?,
            "/api/companion/cancel" => state.companion.cancel(id)?,
            "/api/companion/forget" => state.companion.forget(tv)?,
            "/api/companion/state" => state.companion.state(tv)?,
            "/api/companion/command" => state.companion.command(tv, value)?,
            "/api/companion/resolve" => state.companion.resolve(
                tv,
                reference(state, value["channel_id"].as_str().unwrap_or(""))?,
            )?,
            "/api/companion/start" => state.companion.start_watch(
                id,
                &reference(state, value["channel_id"].as_str().unwrap_or(""))?,
                value["position"].as_u64().unwrap_or(0),
            )?,
            "/api/companion/transfer" => state.companion.transfer(
                tv,
                state.catalog.personal.clone(),
                value["source"].as_str().unwrap_or(""),
            )?,
            "/api/companion/import" => state
                .companion
                .import(id, value["name"].as_str().unwrap_or(""))?,
            _ => return Err(anyhow!("Comando companion non disponibile.")),
        })
    })();
    personal_api::respond_result(request, result);
}
