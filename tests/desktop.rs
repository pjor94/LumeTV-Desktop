// SPDX-License-Identifier: GPL-3.0-or-later
use anyhow::Result;
use lume_player::{
    catalog::{CatalogService, BUNDLED_PLAYLIST},
    desktop::DesktopServer,
    guide::GuideService,
    report::now_ms,
};
use reqwest::blocking::Client;
use serde_json::{json, Value};

fn public_guide_fixture(
    directory: &std::path::Path,
    catalog: std::sync::Arc<CatalogService>,
) -> Result<()> {
    let service = GuideService::with_catalog(directory, catalog)?;
    let now = chrono::Utc::now();
    let start = (now - chrono::Duration::minutes(20)).format("%Y%m%d%H%M%S +0000");
    let stop = (now + chrono::Duration::minutes(40)).format("%Y%m%d%H%M%S +0000");
    let xml = format!(
        r#"<tv><channel id="Rai1.it"><display-name>Rai 1</display-name></channel><channel id="Extra.it"><display-name>Esteso</display-name><icon src="https://images.example.org/extra.png"/></channel><programme channel="Rai1.it" start="{start}" stop="{stop}"><title>Programma Rai</title></programme><programme channel="Extra.it" start="{start}" stop="{stop}"><title>Programma EPG completo</title></programme></tv>"#
    );
    service.import_xml(std::io::Cursor::new(&xml), now_ms())?;
    let extra=xml.replace("</tv>",&format!(r#"<channel id="Sky.Uno.it"><display-name>Sky Uno</display-name><icon src="https://guidatv.sky.it/logo/uno.png"/></channel><programme channel="Sky.Uno.it" start="{start}" stop="{stop}"><title>Programma EPGShare IT1</title></programme></tv>"#));
    service.import_epgshare_xml(std::io::Cursor::new(extra), now_ms())?;
    Ok(())
}

#[test]
fn full_public_epg_and_cached_logos_reach_personal_apis_without_exposing_or_replacing_list_data(
) -> Result<()> {
    use base64::Engine;
    use sha2::Digest;
    let directory =
        std::env::temp_dir().join(format!("lume-metadata-api-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory)?;
    std::fs::write(
        directory.join("catalog.json"),
        serde_json::to_vec(&json!({"playlist":BUNDLED_PLAYLIST,"last_success":now_ms()}))?,
    )?;
    std::fs::write(
        directory.join("radio-catalog.json"),
        serde_json::to_vec(
            &json!({"text":lume_player::radio::BUNDLED,"updated_at":now_ms(),"attempted":now_ms(),"next_retry_at":0}),
        )?,
    )?;
    public_guide_fixture(&directory, CatalogService::new(&directory)?)?;
    let server = DesktopServer::start(&directory)?;
    let token = server.launch_url.split_once('#').unwrap().1;
    let client = Client::builder().no_proxy().build()?;
    let post = |path: &str, value: &Value| -> Result<Value> {
        Ok(client
            .post(format!("{}{path}", server.origin))
            .header("X-Lume-Token", token)
            .json(value)
            .send()?
            .error_for_status()?
            .json()?)
    };
    let job = post(
        "/api/library/prepare",
        &json!({"name":"Metadati sintetici","settings":{}}),
    )?;
    let id = job["id"].as_str().unwrap();
    let text="#EXTM3U\n#EXTINF:-1 tvg-id=\"Extra.it\",Esteso\n#EXTVLCOPT:http-user-agent=private-synthetic-agent\nhttps://private.example/live.mpd?token=private-fixture\n#EXTINF:-1 tvg-id=\"Extra.it\" tvg-logo=\"https://private.example/provided.png\",Esteso alternativo\nhttps://private.example/second.mpd\n#EXTINF:-1 tvg-id=\"Sky.Uno.it\",Sky Uno\nhttps://private.example/sky.mpd\n";
    client
        .post(format!("{}/api/library/upload/{id}", server.origin))
        .header("X-Lume-Token", token)
        .body(text)
        .send()?
        .error_for_status()?;
    let wait = |phase: &str| -> Result<()> {
        for _ in 0..200 {
            let value = post("/api/library/job", &json!({"id":id}))?;
            if value["phase"] == phase {
                return Ok(());
            }
            assert_ne!(value["phase"], "error");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        anyhow::bail!("metadata_fixture_timeout")
    };
    wait("ready")?;
    post("/api/library/commit", &json!({"id":id}))?;
    wait("done")?;
    let page = post(
        "/api/library/catalog",
        &json!({"kind":"tv","content":"live","limit":200}),
    )?;
    assert_eq!(page["total"], 3);
    assert_eq!(page["rows"][0]["now"]["title"], "Programma EPG completo");
    assert_eq!(page["rows"][0]["guide_origin"], "lume");
    assert!(!page.to_string().contains("private-fixture"));
    assert!(!page.to_string().contains("private-synthetic-agent"));
    assert!(!page.to_string().contains("images.example.org"));
    let channel = page["rows"][0]["id"].as_str().unwrap();
    let window = post(
        "/api/library/guide/window",
        &json!({"id":channel,"from":now_ms()-3_600_000,"until":now_ms()+3_600_000}),
    )?;
    assert_eq!(window["rows"][0]["title"], "Programma EPG completo");
    assert_eq!(window["rows"][0]["archive"], false);
    let sky = page["rows"][2]["id"].as_str().unwrap();
    assert_eq!(page["rows"][2]["guide_origin"], "epgshare");
    assert_eq!(page["rows"][2]["now"]["title"], "Programma EPGShare IT1");
    let sky_window = post(
        "/api/library/guide/window",
        &json!({"id":sky,"from":now_ms()-3_600_000,"until":now_ms()+3_600_000}),
    )?;
    assert_eq!(sky_window["origin"], "epgshare");
    assert_eq!(sky_window["rows"][0]["archive"], false);
    let png = base64::engine::general_purpose::STANDARD
        .decode(include_str!("fixtures/guide-logo.png.base64").trim())?;
    std::fs::create_dir_all(directory.join("artwork/public"))?;
    let key = format!(
        "{:x}.img",
        sha2::Sha256::digest("https://images.example.org/extra.png")
    );
    std::fs::write(directory.join("artwork/public").join(key), &png)?;
    let sky_key = format!(
        "{:x}.img",
        sha2::Sha256::digest("https://guidatv.sky.it/logo/uno.png")
    );
    std::fs::write(directory.join("artwork/public").join(sky_key), &png)?;
    let image = client
        .get(format!("{}/art/channel/{channel}", server.origin))
        .send()?
        .error_for_status()?;
    assert_eq!(image.headers()["Content-Type"], "image/png");
    assert_eq!(image.bytes()?.as_ref(), png.as_slice());
    let sky_logo = client
        .get(format!("{}/art/channel/{sky}", server.origin))
        .send()?
        .error_for_status()?;
    assert_eq!(sky_logo.headers()["Content-Type"], "image/png");
    assert_eq!(sky_logo.bytes()?.as_ref(), png.as_slice());
    drop(server);
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

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
    public_guide_fixture(&directory, catalogue)?;
    std::fs::write(
        directory.join("radio-catalog.json"),
        serde_json::to_vec(
            &json!({"text":lume_player::radio::BUNDLED,"updated_at":now_ms(),"attempted":now_ms(),"next_retry_at":0}),
        )?,
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
    assert_eq!(
        initial["channels"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["kind"] == "tv")
            .count(),
        126
    );
    assert_eq!(
        initial["channels"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["kind"] == "radio")
            .count(),
        1310
    );
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
    for path in ["/api/radio/status", "/api/radios/refresh"] {
        let request = if path.ends_with("refresh") {
            client
                .post(format!("{}{path}", server.origin))
                .json(&json!({}))
        } else {
            client.get(format!("{}{path}", server.origin))
        };
        assert_eq!(request.send()?.status().as_u16(), 403);
    }
    assert_eq!(
        client
            .get(format!("{}/audio/invalid", server.origin))
            .send()?
            .status()
            .as_u16(),
        404
    );
    assert_eq!(
        client
            .get(format!("{}/audio/invalid", server.origin))
            .header("Origin", "https://untrusted.test")
            .send()?
            .status()
            .as_u16(),
        403
    );
    assert!(component.get("downloaded_bytes").is_none());
    let html = client.get(format!("{}/", server.origin)).send()?.text()?;
    assert!(html.contains("/app.js"));
    assert!(!html.contains("/src/main.tsx"));
    for path in [
        "/api/updates",
        "/api/updates/download",
        "/api/updates/install",
        "/api/updates/settings",
        "/api/companion/discover",
        "/api/companion/pair",
        "/api/library/backup/export",
        "/api/library/backup/upload/untrusted",
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
    // Exercise the real authenticated import API without opening a provider connection.
    let post = |path: &str, body: Value| -> Result<Value> {
        let response = client
            .post(format!("{}{path}", server.origin))
            .header("X-Lume-Token", token)
            .json(&body)
            .send()?;
        anyhow::ensure!(
            response.status().is_success(),
            "{path} returned {}",
            response.status()
        );
        Ok(response.json()?)
    };
    for path in [
        "/api/library",
        "/api/library/catalog",
        "/api/library/prepare",
        "/api/library/guide/window",
        "/api/library/watch",
        "/api/library/video/page",
        "/api/library/video/detail",
        "/api/library/video/settings",
        "/api/library/video/metadata",
    ] {
        assert_eq!(
            client
                .post(format!("{}{path}", server.origin))
                .json(&json!({}))
                .send()?
                .status()
                .as_u16(),
            403
        );
    }
    let settings = lume_player::personal::Settings::default();
    let prepared = post(
        "/api/library/prepare",
        json!({"name":"Lista personale","settings":settings}),
    )?;
    let job = prepared["id"].as_str().unwrap();
    let playlist="#EXTM3U x-tvg-url=\"https://guide.example/tv.xml?private=guide\"\n#EXTINF:-1 tvg-id=\"live\" group-title=\"Notizie\",Diretta\nhttps://media.example/live.m3u8?private=live\n#EXTINF:120 tvg-id=\"film\" group-title=\"Cinema\",Un film\n#EXTVLCOPT:http-referrer=https://private.example/\nhttps://media.example/film.mp4?private=film\n";
    assert_eq!(
        client
            .post(format!("{}/api/library/upload/{job}", server.origin))
            .body(playlist)
            .send()?
            .status()
            .as_u16(),
        403
    );
    assert!(client
        .post(format!("{}/api/library/upload/{job}", server.origin))
        .header("X-Lume-Token", token)
        .body(playlist)
        .send()?
        .status()
        .is_success());
    let wait = |phase: &str| -> Result<Value> {
        let started = std::time::Instant::now();
        loop {
            let value = post("/api/library/job", json!({"id":job}))?;
            if value["phase"] == phase {
                return Ok(value);
            }
            anyhow::ensure!(
                value["phase"] != "error" && started.elapsed() < std::time::Duration::from_secs(15),
                "Import failed or timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    };
    let preview = wait("ready")?;
    assert_eq!(preview["report"]["count"], 2);
    assert_eq!(preview["report"]["vod"], 1);
    assert!(!preview.to_string().contains("private="));
    assert_eq!(post("/api/library/catalog", json!({}))?["total"], 0);
    post("/api/library/commit", json!({"id":job}))?;
    let committed = wait("done")?;
    let source = committed["source_id"].as_str().unwrap();
    assert_eq!(
        client
            .post(format!("{}/api/library/home", server.origin))
            .send()?
            .status()
            .as_u16(),
        403
    );
    let home = post("/api/library/home", json!({"source":source}))?;
    assert_eq!(home["total"], 1);
    assert_eq!(home["featured"]["name"], "Diretta");
    assert_eq!(home["recent"], json!([]));
    assert!(!home.to_string().contains("private="));
    let catalog = post(
        "/api/library/catalog",
        json!({"source":source,"content":"vod"}),
    )?;
    assert_eq!(catalog["total"], 1);
    assert_eq!(catalog["rows"][0]["name"], "Un film");
    assert!(!catalog.to_string().contains("https://"));
    let channel = catalog["rows"][0]["id"].as_str().unwrap();
    let video_page = post("/api/library/video/page", json!({"source":source}))?;
    assert_eq!(video_page["total"], 1);
    assert!(!video_page.to_string().contains("private="));
    assert!(!video_page.to_string().contains("https://"));
    let video_detail = post("/api/library/video/detail", json!({"id":channel}))?;
    assert_eq!(video_detail["target"], channel);
    assert!(video_detail["next"].is_null());
    assert_eq!(
        post("/api/library/video/settings", json!({}))?["enabled"],
        false
    );
    assert_eq!(
        post("/api/library/video/metadata", json!({"id":channel}))?["status"],
        "disabled"
    );
    post("/api/preferences", json!({"toggle_favorite":channel}))?;
    let removed = post(
        "/api/library/video/action",
        json!({"id":channel,"action":"saved","enabled":false}),
    )?;
    assert!(!removed["preferences"]["favorites"]
        .as_array()
        .unwrap()
        .iter()
        .any(|id| id == channel));
    assert_eq!(
        post("/api/library/video/page", json!({"shelf":"saved"}))?["total"],
        0
    );
    post("/api/preferences", json!({"toggle_favorite":channel}))?;
    let library: Value = client
        .get(format!("{}/api/library", server.origin))
        .header("X-Lume-Token", token)
        .send()?
        .json()?;
    assert_eq!(library["favorite_count"], 0);
    assert_eq!(
        post("/api/library/catalog", json!({"favorites":true}))?["total"],
        1
    );
    post(
        "/api/library/watch",
        json!({"id":channel,"mode":"vod","position":42,"duration":120}),
    )?;
    let playing = post("/api/play", json!({"channel_id":channel}))?;
    assert_eq!(playing["stream"]["mode"], "vod");
    assert_eq!(playing["stream"]["position"], 42.0);
    let revision = playing["stream"]["watch_revision"].as_u64().unwrap();
    post(
        "/api/library/video/action",
        json!({"id":channel,"action":"completed","enabled":true}),
    )?;
    post(
        "/api/library/watch",
        json!({"id":channel,"mode":"vod","position":44,"duration":120,"revision":revision}),
    )?;
    assert_eq!(
        post("/api/library/video/detail", json!({"id":channel}))?["item"]["completed"],
        true
    );
    assert!(playing["stream"]["url"]
        .as_str()
        .unwrap()
        .starts_with(&format!("{}/stream/", server.origin)));
    assert!(!playing.to_string().contains("private="));
    let played = post("/api/played", json!({"playback_id":playing["playback_id"]}))?;
    assert_eq!(played["recent"], initial["preferences"]["recent"]);
    post("/api/stop", json!({"playback_id":playing["playback_id"]}))?;
    post("/api/preferences", json!({"toggle_favorite":channel}))?;
    let detail = post("/api/library/source", json!({"id":source}))?;
    post(
        "/api/library/delete",
        json!({"id":source,"revision":detail["revision"]}),
    )?;
    assert_eq!(post("/api/library/catalog", json!({}))?["total"], 0);
    drop(server);
    let stored: Value =
        serde_json::from_slice(&std::fs::read(directory.join("preferences.json"))?)?;
    assert_eq!(stored["favorites"], json!(["rai1"]));
    std::fs::remove_dir_all(directory)?;
    Ok(())
}
