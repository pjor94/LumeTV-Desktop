// SPDX-License-Identifier: GPL-3.0-or-later
use anyhow::Result;
use lume_player::{
    catalog::{CatalogService, BUNDLED_PLAYLIST},
    desktop::DesktopServer,
    guide::GuideSnapshot,
    report::now_ms,
};
use reqwest::blocking::Client;
use serde_json::{json, Value};

#[test]
fn desktop_authentication_preferences_and_stale_playback_are_enforced_without_provider_requests(
) -> Result<()> {
    let directory =
        std::env::temp_dir().join(format!("lume-desktop-fixture-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory)?;
    std::fs::write(
        directory.join("catalog.json"),
        serde_json::to_vec(&json!({"playlist":BUNDLED_PLAYLIST,"last_success":now_ms()}))?,
    )?;
    let catalogue = CatalogService::new(&directory)?;
    std::fs::write(
        directory.join("guide.json"),
        serde_json::to_vec(&GuideSnapshot {
            updated_at: now_ms(),
            catalog_signature: catalogue.guide_signature(),
            ..Default::default()
        })?,
    )?;
    let server = DesktopServer::start(&directory)?;
    let token = server.launch_url.split_once('#').unwrap().1;
    let client = Client::builder().no_proxy().build()?;
    assert_eq!(
        client
            .get(format!("{}/api/bootstrap", server.origin))
            .send()?
            .status()
            .as_u16(),
        403
    );
    assert_eq!(
        client
            .get(format!("{}/api/bootstrap", server.origin))
            .header("X-Lume-Token", token)
            .header("Origin", "https://untrusted.test")
            .send()?
            .status()
            .as_u16(),
        403
    );
    let initial: Value = client
        .get(format!("{}/api/bootstrap", server.origin))
        .header("X-Lume-Token", token)
        .send()?
        .json()?;
    assert_eq!(initial["channels"].as_array().unwrap().len(), 126);
    for path in ["/api/widevine", "/api/widevine/update"] {
        let response = if path.ends_with("/update") {
            client
                .post(format!("{}{path}", server.origin))
                .json(&json!({}))
                .send()?
        } else {
            client.get(format!("{}{path}", server.origin)).send()?
        };
        assert_eq!(response.status().as_u16(), 403);
    }
    let component: Value = client
        .get(format!("{}/api/widevine", server.origin))
        .header("X-Lume-Token", token)
        .send()?
        .json()?;
    assert_eq!(component["phase"], "unavailable");
    assert!(component.get("downloaded_bytes").is_none());
    let html = client.get(format!("{}/", server.origin)).send()?.text()?;
    assert!(html.contains("/app.js"));
    assert!(!html.contains("/src/main.tsx"));
    for path in [
        "/api/updates",
        "/api/updates/download",
        "/api/updates/install",
        "/api/updates/settings",
    ] {
        let request = if path == "/api/updates" {
            client.get(format!("{}{path}", server.origin))
        } else {
            client
                .post(format!("{}{path}", server.origin))
                .json(&json!({}))
        };
        assert_eq!(request.send()?.status().as_u16(), 403);
    }
    let updates: Value = client
        .get(format!("{}/api/updates", server.origin))
        .header("X-Lume-Token", token)
        .send()?
        .json()?;
    assert_eq!(updates["phase"], "idle");
    assert!(updates.get("destination").is_none());
    let invalid_update = client
        .post(format!("{}/api/updates/install", server.origin))
        .header("X-Lume-Token", token)
        .json(&json!({"destination":"/tmp/ignored","url":"https://untrusted.test"}))
        .send()?;
    assert_eq!(invalid_update.status().as_u16(), 409);
    for channel in initial["channels"].as_array().unwrap() {
        for private in ["key", "headers", "logo"] {
            assert!(channel.get(private).is_none());
        }
    }
    let catalog: Value = client
        .get(format!("{}/api/catalog", server.origin))
        .header("X-Lume-Token", token)
        .send()?
        .json()?;
    assert_eq!(catalog["channels"], initial["channels"]);
    assert_eq!(catalog["revision"], initial["catalog"]["revision"]);
    assert_eq!(catalog["loading"], false);
    assert_eq!(
        client
            .post(format!("{}/api/catalog/refresh", server.origin))
            .send()?
            .status()
            .as_u16(),
        403
    );
    let preferences: Value = client
        .post(format!("{}/api/preferences", server.origin))
        .header("X-Lume-Token", token)
        .json(&json!({"toggle_favorite":"rai1","theme":"light"}))
        .send()?
        .json()?;
    assert_eq!(preferences["favorites"], json!(["rai1"]));
    assert_eq!(preferences["theme"], "light");
    let invalid = client
        .post(format!("{}/api/play", server.origin))
        .header("X-Lume-Token", token)
        .json(&json!({"channel_id":"https://untrusted.test/stream"}))
        .send()?;
    assert_eq!(invalid.status().as_u16(), 400);
    let stale = client
        .post(format!("{}/api/fetch", server.origin))
        .header("X-Lume-Token", token)
        .header("X-Lume-Playback-Id", "expired")
        .json(&json!({"kind":"license", "url":"https://untrusted.test"}))
        .send()?;
    assert_eq!(stale.status().as_u16(), 409);
    drop(server);
    let stored: Value =
        serde_json::from_slice(&std::fs::read(directory.join("preferences.json"))?)?;
    assert_eq!(stored["favorites"], json!(["rai1"]));
    std::fs::remove_dir_all(directory)?;
    Ok(())
}
