use super::*;
use serde_json::Value;
use std::{
    io::Cursor,
    thread,
    time::{Duration, Instant},
};

pub(super) struct Fixture {
    pub(super) root: std::path::PathBuf,
    pub(super) library: Arc<Library>,
}
impl Fixture {
    pub(super) fn new() -> Self {
        let root = std::env::temp_dir().join(format!("lume-library-{}", uuid::Uuid::new_v4()));
        let library = Library::new(&root).unwrap();
        Self { root, library }
    }
    fn verify(&self, text: &str, source: Option<&str>) -> String {
        let old = source.map(|id| self.library.source(id).unwrap());
        let config = ImportConfig {
            source_id: source.unwrap_or("").into(),
            revision: old.as_ref().map_or(0, |s| s.revision),
            name: "Lista di prova".into(),
            url: None,
            settings: old.map(|s| s.settings).unwrap_or_default(),
        };
        let job = self.library.prepare(config, false).unwrap();
        let id = job["id"].as_str().unwrap().to_owned();
        self.library
            .upload(&id, &mut Cursor::new(text.as_bytes()))
            .unwrap();
        self.wait(&id, "ready");
        id
    }
    fn wait(&self, id: &str, phase: &str) -> Value {
        let start = Instant::now();
        let mut reported = 0;
        loop {
            let status = self.library.job(id).unwrap();
            let count = status["entries"].as_u64().unwrap_or(0);
            if count >= reported + 100000 {
                eprintln!("{count} voci verificate");
                reported = count;
            }
            let current = status["phase"].as_str().unwrap();
            if current == phase {
                return status;
            }
            assert!(!["error", "cancelled"].contains(&current), "{status}");
            assert!(
                start.elapsed() < Duration::from_secs(600),
                "Import timed out: {status}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
    pub(super) fn import(&self, text: &str, source: Option<&str>) -> String {
        let id = self.verify(text, source);
        self.library.commit(&id).unwrap();
        self.wait(&id, "done")["source_id"].as_str().unwrap().into()
    }
    pub(super) fn rows(&self, source: &str) -> Value {
        self.library
            .catalog(
                &CatalogQuery {
                    source: source.into(),
                    limit: 200,
                    ..Default::default()
                },
                &[],
            )
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn playlist(url: &str) -> String {
    format!("#EXTM3U\n#EXTINF:-1 tvg-id=\"one\" group-title=\"Notizie\",Primo\n{url}\n")
}

#[test]
fn home_keeps_recent_live_tv_in_history_order_and_in_the_selected_enabled_source() {
    let f = Fixture::new();
    let mut text = "#EXTM3U\n".to_owned();
    for i in 0..8 {
        text.push_str(&format!(
            "#EXTINF:-1,Canale {i}\nhttps://media.example/{i}.m3u8?private=test\n"
        ));
    }
    text.push_str("#EXTINF:120,Un film\nhttps://media.example/film.mp4\n#EXTINF:-1 radio=\"true\",Una radio\nhttps://media.example/audio.mp3\n");
    let source = f.import(&text, None);
    let rows = f.rows(&source);
    let ids = rows["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    let other = f.import(&playlist("https://media.example/other.m3u8"), None);
    let other_id = f.rows(&other)["rows"][0]["id"].as_str().unwrap().to_owned();
    let history = std::iter::once(other_id.clone())
        .chain(ids.iter().rev().cloned())
        .collect::<Vec<_>>();
    let home = f.library.home(&source, &history).unwrap();
    assert_eq!(home["total"], 8);
    assert_eq!(home["featured"]["id"], ids[7]);
    assert_eq!(home["recent"].as_array().unwrap().len(), 5);
    for (i, item) in home["recent"].as_array().unwrap().iter().enumerate() {
        assert_eq!(item["id"], ids[7 - i]);
    }
    assert!(!home.to_string().contains("https://"));
    assert!(!home.to_string().contains("private="));
    let fresh = f.library.home(&other, &ids).unwrap();
    assert_eq!(fresh["featured"]["id"], other_id);
    assert_eq!(fresh["recent"], serde_json::json!([]));
    let mut long = vec!["unavailable".to_owned(); 40];
    long.push(ids[7].clone());
    assert_eq!(
        f.library.home(&source, &long).unwrap()["recent"],
        serde_json::json!([])
    );
    let detail = f.library.source(&source).unwrap();
    f.library
        .edit(
            &source,
            detail.revision,
            &detail.name,
            false,
            detail.settings,
        )
        .unwrap();
    let disabled = f.library.home(&source, &history).unwrap();
    assert_eq!(disabled["total"], 0);
    assert!(disabled["featured"].is_null());
    assert_eq!(disabled["recent"], serde_json::json!([]));
}

#[test]
fn import_is_private_atomic_and_preserves_identity_and_watch_position() {
    let f = Fixture::new();
    let source = f.import(
        &playlist("https://media.example/one.mp4?private=example"),
        None,
    );
    let rows = f.rows(&source);
    let id = rows["rows"][0]["id"].as_str().unwrap().to_owned();
    let public = rows.to_string();
    for forbidden in [
        "private=example",
        "https://",
        "headers",
        "properties",
        "metadata",
    ] {
        assert!(!public.contains(forbidden), "{forbidden}");
    }
    f.library.watch(&id, "vod", "", 42.0, 120.0, false).unwrap();
    f.import(
        &playlist("https://media.example/replaced.mp4"),
        Some(&source),
    );
    assert_eq!(f.rows(&source)["rows"][0]["id"], id);
    let entry = f.library.entry(&id).unwrap().1;
    assert!(entry.url.ends_with("replaced.mp4"));
    let reopened = Library::new(&f.root).unwrap();
    assert!(reopened.channel(&id).is_some());
}
#[test]
fn source_revisions_prevent_deleted_or_stale_imports_from_reappearing() {
    let f = Fixture::new();
    let source = f.import(&playlist("https://media.example/one.m3u8"), None);
    let job = f.verify(&playlist("https://media.example/two.m3u8"), Some(&source));
    let old = f.library.source(&source).unwrap();
    f.library.delete(&source, old.revision).unwrap();
    assert!(f.library.commit(&job).is_err());
    let start = Instant::now();
    loop {
        let state = f.library.job(&job).unwrap();
        if ["error", "cancelled"].contains(&state["phase"].as_str().unwrap()) {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(10));
    }
    assert!(f.library.snapshot()["sources"]
        .as_array()
        .unwrap()
        .is_empty());
}
#[test]
fn ambiguous_tvg_ids_fall_back_to_unique_urls() {
    let f = Fixture::new();
    let text="#EXTM3U\n#EXTINF:-1 tvg-id=\"same\",Prima\nhttps://media.example/a.m3u8\n#EXTINF:-1 tvg-id=\"same\",Seconda\nhttps://media.example/b.m3u8\n";
    let source = f.import(text, None);
    let mut old = f.rows(&source);
    f.import(text, Some(&source));
    let mut updated = f.rows(&source);
    for (before, after) in old["rows"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .zip(updated["rows"].as_array_mut().unwrap())
    {
        assert_eq!(before["artwork_revision"], "1");
        assert_eq!(after["artwork_revision"], "2");
        before.as_object_mut().unwrap().remove("artwork_revision");
        after.as_object_mut().unwrap().remove("artwork_revision");
    }
    assert_eq!(old["rows"], updated["rows"]);
}
#[test]
fn paging_filtering_favorites_and_zapping_use_the_full_database() {
    let f = Fixture::new();
    let mut text = "#EXTM3U\n".to_owned();
    for i in 0..1250 {
        text.push_str(&format!("#EXTINF:-1 tvg-id=\"id{i}\" group-title=\"Gruppo {}\",Canale {i}\nhttps://media.example/{i}.m3u8\n",i%5));
    }
    let source = f.import(&text, None);
    let page = f
        .library
        .catalog(
            &CatalogQuery {
                source: source.clone(),
                offset: 1200,
                limit: 200,
                ..Default::default()
            },
            &[],
        )
        .unwrap();
    assert_eq!(page["total"], 1250);
    assert_eq!(page["rows"].as_array().unwrap().len(), 50);
    let id = page["rows"][49]["id"].as_str().unwrap();
    assert_eq!(
        f.library
            .next(id, 1, &[], false)
            .unwrap()
            .unwrap()
            .channel
            .name,
        "Canale 0"
    );
    let favorite = f
        .library
        .catalog(
            &CatalogQuery {
                favorites: true,
                ..Default::default()
            },
            &[id.into()],
        )
        .unwrap();
    assert_eq!(favorite["total"], 1);
    let groups = f
        .library
        .groups(&CatalogQuery {
            source,
            query: "Gruppo".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(groups["total"], 5);
}
#[test]
fn http_and_credentials_are_scoped_and_guide_changes_keep_video_valid() {
    let f = Fixture::new();
    let source = f.import(&playlist("https://media.example/live.m3u8"), None);
    let old = f.library.source(&source).unwrap();
    let context = f.library.context(&old).unwrap();
    let mut settings = old.settings.clone();
    settings.guide.enabled = true;
    settings.guide.urls = vec!["https://guide.example/tv.xml".into()];
    f.library
        .edit(&source, old.revision, &old.name, true, settings.clone())
        .unwrap();
    assert!(context.check().is_ok());
    settings.allow_http = true;
    let revised = f.library.source(&source).unwrap();
    f.library
        .edit(&source, revised.revision, &revised.name, true, settings)
        .unwrap();
    assert!(context.check().is_err());
    assert!(network::validate_url("http://media.example/live", false).is_err());
    assert!(network::validate_url("https://user:pass@media.example/live", true).is_err());
    assert!(network::validate_url("https://127.0.0.1/live", true).is_err());
    assert!(network::validate_headers(&[("Host".into(), "example".into())].into()).is_err());
    assert!(
        network::validate_headers(&[("Authorization".into(), "line\r\nbad".into())].into())
            .is_err()
    );
}
#[test]
fn simple_and_legacy_encoded_playlists_are_supported() {
    let f = Fixture::new();
    let mut config = ImportConfig {
        name: "Semplice".into(),
        ..Default::default()
    };
    config.settings.format = "simple".into();
    config.settings.encoding = "latin-1".into();
    let job = f.library.prepare(config, false).unwrap();
    let id = job["id"].as_str().unwrap();
    f.library
        .upload(id, &mut Cursor::new(b"https://media.example/cafe.mp4\n"))
        .unwrap();
    let state = f.wait(id, "ready");
    assert_eq!(state["report"]["count"], 1);
    f.library.cancel(id).unwrap();
    assert_eq!(f.library.job(id).unwrap()["phase"], "cancelled");
    assert!(f.library.snapshot()["sources"]
        .as_array()
        .unwrap()
        .is_empty());
}
#[test]
fn unsupported_or_truncated_input_keeps_the_previous_snapshot() {
    let f = Fixture::new();
    let source = f.import(&playlist("https://media.example/one.m3u8"), None);
    let before = f.rows(&source);
    let old = f.library.source(&source).unwrap();
    let job = f
        .library
        .prepare(
            ImportConfig {
                source_id: source.clone(),
                revision: old.revision,
                name: old.name,
                url: None,
                settings: old.settings,
            },
            false,
        )
        .unwrap();
    let id = job["id"].as_str().unwrap();
    f.library
        .upload(id, &mut Cursor::new(b"#EXTM3U\n#EXTINF:-1,Incomplete\n"))
        .unwrap();
    let start = Instant::now();
    loop {
        if f.library.job(id).unwrap()["phase"] == "error" {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(f.rows(&source), before);
}
#[test]
fn clearkey_is_normalized_without_exporting_keys_to_the_catalog() {
    let keys = "00112233445566778899aabbccddeeff:ffeeddccbbaa99887766554433221100";
    let parsed: Value = serde_json::from_slice(&drm::clear_keys(keys).unwrap()).unwrap();
    assert_eq!(parsed["keys"].as_array().unwrap().len(), 1);
    assert_eq!(parsed["type"], "temporary");
    assert!(drm::clear_keys("0011:2233").is_err());
    let f = Fixture::new();
    let text=format!("#EXTM3U\n#EXTINF:-1,CENC\n#KODIPROP:inputstream.adaptive.license_type=clearkey\n#KODIPROP:inputstream.adaptive.license_key={keys}\nhttps://media.example/live.mpd\n");
    let source = f.import(&text, None);
    let rows = f.rows(&source);
    let id = rows["rows"][0]["id"].as_str().unwrap();
    let playback = f.library.playback(id, "live", "", None).unwrap();
    assert_eq!(playback.policy.key_system(), Some("org.w3.clearkey"));
    assert_eq!(
        playback
            .policy
            .drm
            .as_ref()
            .unwrap()
            .local_clear_keys()
            .unwrap()
            .unwrap()
            .get("00112233445566778899aabbccddeeff")
            .unwrap(),
        "ffeeddccbbaa99887766554433221100"
    );
    assert!(!rows.to_string().contains("001122"));
    assert_eq!(
        playback
            .policy
            .drm
            .as_ref()
            .unwrap()
            .license(&playback.policy.context, b"challenge")
            .unwrap(),
        drm::clear_keys(keys).unwrap()
    );
}

#[test]
fn rejected_personal_media_counts_as_a_network_failure_without_exposing_upstream_data() {
    let upstream = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let address = upstream.server_addr().to_ip().unwrap();
    let worker = thread::spawn(move || {
        let request = upstream
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        request
            .respond(
                tiny_http::Response::from_string("private upstream body").with_status_code(403),
            )
            .unwrap();
    });
    let f = Fixture::new();
    let source = f.import(&playlist("https://media.example/manifest.mpd"), None);
    let id = f.rows(&source)["rows"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (_, mut entry) = f.library.entry(&id).unwrap();
    entry.url = format!("http://media.example:{}/manifest.mpd", address.port());
    let context = network::Context::fixture(
        &Settings {
            allow_http: true,
            ..Default::default()
        },
        &[("media.example", address)],
    );
    let policy = drm::policy(&entry, context).unwrap();
    let stream = ResolvedStream {
        url: entry.url.clone(),
        mime_type: Some("application/dash+xml".into()),
        headers: Headers::new(),
        drm: None,
    };
    let relay = crate::server::HarnessServer::start_with_policy(
        crate::model::Channel {
            id,
            name: "Synthetic denied channel".into(),
            kind: crate::model::MediaKind::Tv,
            provider: "personal".into(),
            key: String::new(),
            headers: Headers::new(),
        },
        stream,
        30,
        "relay",
        Some(policy),
    )
    .unwrap();
    let token = relay.launch_url.split_once('#').unwrap().1;
    let response = reqwest::blocking::Client::new()
        .post(format!("{}/api/fetch", relay.origin))
        .header("X-Lume-Token", token)
        .json(&serde_json::json!({"kind":"media","url":entry.url,"resource":"media"}))
        .send()
        .unwrap();
    assert_eq!(response.status().as_u16(), 403);
    assert_eq!(response.text().unwrap(), r#"{"error":"upstream_response"}"#);
    assert_eq!(relay.network_summary()["failed_requests"], 1);
    assert_eq!(relay.network_summary()["media_http_successes"], 0);
    worker.join().unwrap();
}

#[test]
fn explicit_user_agent_is_sent_once_and_replaces_the_default() {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let address = server.server_addr().to_ip().unwrap();
    let worker = thread::spawn(move || {
        let request = server
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        let agents = request
            .headers()
            .iter()
            .filter(|h| h.field.equiv("User-Agent"))
            .map(|h| h.value.as_str())
            .collect::<Vec<_>>();
        assert_eq!(agents, vec!["fixture-agent"]);
        request
            .respond(tiny_http::Response::from_string("ok"))
            .unwrap();
    });
    let context = network::Context::fixture(
        &Settings {
            allow_http: true,
            ..Default::default()
        },
        &[("media.example", address)],
    );
    let url = format!("http://media.example:{}/manifest.mpd", address.port());
    let response = context
        .request(
            &url,
            &url,
            Resource::Manifest,
            &[("user-agent".into(), "fixture-agent".into())].into(),
            reqwest::Method::GET,
            None,
        )
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    worker.join().unwrap();
}
#[test]
fn m3u_header_precedence_and_separate_hls_keys_are_preserved() {
    let f = Fixture::new();
    let text="#EXTM3U\n#EXTINF:-1 http-user-agent=\"first\",Header\n#KODIPROP:inputstream.adaptive.common_headers=User-Agent=second\n#EXTHTTP:{\"User-Agent\":\"third\"}\n#KODIPROP:inputstream.adaptive.manifest_headers=Referer=https%3A%2F%2Fmanifest.example\n#KODIPROP:inputstream.adaptive.stream_headers=Origin=https%3A%2F%2Fmedia.example\n#KODIPROP:inputstream.adaptive.license_key=token=example|X-Key=key||\nhttps://media.example/live.m3u8|User-Agent=fourth\n";
    let source = f.import(text, None);
    let rows = f.rows(&source);
    let playback = f
        .library
        .playback(rows["rows"][0]["id"].as_str().unwrap(), "live", "", None)
        .unwrap();
    assert_eq!(playback.policy.media["User-Agent"], "fourth");
    assert_eq!(playback.policy.key_parameters, "token=example");
    assert_eq!(playback.policy.key["X-Key"], "key");
    assert!(!playback.policy.manifest.contains_key("Origin"));
}
#[test]
fn vod_resume_and_completion_are_distinct_from_live_history() {
    let f = Fixture::new();
    let source = f.import(
        "#EXTM3U\n#EXTINF:120,Film\nhttps://media.example/film.mp4\n",
        None,
    );
    let rows = f.rows(&source);
    let id = rows["rows"][0]["id"].as_str().unwrap();
    f.library.watch(id, "vod", "", 42.0, 120.0, false).unwrap();
    let playback = f.library.playback(id, "live", "", None).unwrap();
    assert_eq!(playback.mode, "vod");
    assert_eq!(playback.position, 42.0);
    f.library.watch(id, "vod", "", 120.0, 120.0, true).unwrap();
    assert_eq!(
        f.library.playback(id, "vod", "", None).unwrap().position,
        0.0
    );
    assert!(f.library.watch(id, "live", "", 12.0, 0.0, false).is_err());
}
#[test]
fn xmltv_partial_dates_timezones_and_dst_are_understood() {
    let zone = "Europe/Rome".parse().unwrap();
    assert!(guide::timestamp("202611", zone).is_some());
    assert_eq!(
        guide::timestamp("20261113120000 +0100", zone),
        guide::timestamp("20261113110000 +0000", zone)
    );
    assert!(guide::timestamp("20260230120000", zone).is_none());
    assert!(guide::timestamp("20261113120000 +9999", zone).is_none());
}
#[test]
fn archive_url_generation_preserves_signed_queries_and_refuses_unknown_templates() {
    let f = Fixture::new();
    let source=f.import("#EXTM3U\n#EXTINF:-1 catchup=\"append\" catchup-days=\"7\" catchup-source=\"utc={utc}&duration={duration:60}\",Replay\nhttps://media.example/live.m3u8?sig=a%2Fb\n",None);
    let rows = f.rows(&source);
    let (_, entry) = f
        .library
        .entry(rows["rows"][0]["id"].as_str().unwrap())
        .unwrap();
    let now = crate::report::now_ms();
    let event = guide::GuideEvent {
        programme: crate::guide::Programme {
            start: now - 3600000,
            end: now - 1800000,
            title: "Test".into(),
            ..Default::default()
        },
        catchup_id: String::new(),
    };
    let archive = catchup::build(&entry, &GuideSettings::default(), &event, now, 0.0).unwrap();
    assert!(archive.url.contains("sig=a%2Fb&utc="));
    assert!(archive.url.contains("duration=30"));
    assert!(archive.regenerate);
    let mut entry = entry;
    entry
        .metadata
        .attributes
        .insert("catchup-source".into(), "?unknown={SID}".into());
    assert!(catchup::build(&entry, &GuideSettings::default(), &event, now, 0.0).is_err());
}

#[test]
#[ignore = "Large on-disk capacity check; run explicitly before release"]
fn million_entry_library_keeps_queries_bounded() {
    let f = Fixture::new();
    let mut text = String::from("#EXTM3U\n");
    for i in 0..500000 {
        text.push_str(&format!("#EXTINF:-1 tvg-id=\"t{i}\" group-title=\"G{}\",Canale {i}\nhttps://media.example/{i}.m3u8\n",i%20));
    }
    let source = f.import(&text, None);
    let last = f
        .library
        .catalog(
            &CatalogQuery {
                source: source.clone(),
                offset: 499800,
                limit: 200,
                ..Default::default()
            },
            &[],
        )
        .unwrap();
    assert_eq!(last["total"], 500000);
    assert_eq!(last["rows"].as_array().unwrap().len(), 200);
    assert_eq!(last["rows"][199]["name"], "Canale 499999");
    f.import(&text, None);
    let all = f
        .library
        .catalog(
            &CatalogQuery {
                offset: 999800,
                limit: 200,
                ..Default::default()
            },
            &[],
        )
        .unwrap();
    assert_eq!(all["total"], 1000000);
    assert_eq!(all["rows"].as_array().unwrap().len(), 200);
    let extra = f.verify(&playlist("https://media.example/extra.m3u8"), None);
    f.library.commit(&extra).unwrap();
    assert!(f.wait(&extra, "error")["error"]
        .as_str()
        .unwrap()
        .contains("un milione"));
    assert_eq!(f.library.snapshot()["sources"].as_array().unwrap().len(), 2);
}

#[test]
fn redirects_strip_access_and_cookie_sessions_are_separate_for_each_role() {
    use std::sync::Mutex;
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let address = server.server_addr().to_ip().unwrap();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let copy = observed.clone();
    let worker = thread::spawn(move || {
        for _ in 0..5 {
            let request = server
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap();
            let headers = request
                .headers()
                .iter()
                .map(|h| (h.field.to_string().to_lowercase(), h.value.to_string()))
                .collect::<BTreeMap<_, _>>();
            let path = request.url().to_owned();
            copy.lock().unwrap().push((path.clone(), headers));
            let response = if path == "/redirect" {
                tiny_http::Response::from_string("")
                    .with_status_code(302)
                    .with_header(
                        tiny_http::Header::from_bytes(
                            "Location",
                            format!("http://other.example:{}/land", address.port()),
                        )
                        .unwrap(),
                    )
            } else if path == "/cookie" {
                tiny_http::Response::from_string("ok").with_header(
                    tiny_http::Header::from_bytes("Set-Cookie", "session=fixture; Path=/").unwrap(),
                )
            } else {
                tiny_http::Response::from_string("ok")
            };
            request.respond(response).unwrap();
        }
    });
    let settings = Settings {
        allow_http: true,
        cookies: true,
        ..Default::default()
    };
    let context = network::Context::fixture(
        &settings,
        &[("media.example", address), ("other.example", address)],
    );
    let root = format!("http://media.example:{}", address.port());
    let headers = [
        ("Authorization".into(), "Bearer fixture".into()),
        ("X-Private".into(), "fixture".into()),
        ("User-Agent".into(), "Fixture".into()),
    ]
    .into();
    assert!(context
        .request(
            &root,
            &format!("{root}/redirect"),
            Resource::Media,
            &headers,
            reqwest::Method::GET,
            None
        )
        .unwrap()
        .status()
        .is_success());
    for (path, role) in [
        ("cookie", Resource::Media),
        ("media", Resource::Media),
        ("license", Resource::License),
    ] {
        context
            .request(
                &root,
                &format!("{root}/{path}"),
                role,
                &Headers::new(),
                reqwest::Method::GET,
                None,
            )
            .unwrap();
    }
    worker.join().unwrap();
    let observed = observed.lock().unwrap();
    assert_eq!(observed[0].1["authorization"], "Bearer fixture");
    assert!(!observed[1].1.contains_key("authorization"));
    assert!(!observed[1].1.contains_key("x-private"));
    assert_eq!(observed[1].1["user-agent"], "Fixture");
    assert!(observed[3]
        .1
        .get("cookie")
        .is_some_and(|v| v.contains("session=fixture")));
    assert!(!observed[4].1.contains_key("cookie"));
}

#[test]
fn license_wrappers_and_explicit_json_response_paths_are_translated() {
    use base64::Engine;
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let address = server.server_addr().to_ip().unwrap();
    let worker = thread::spawn(move || {
        let mut request = server
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert_eq!(request.method(), &tiny_http::Method::Post);
        let mut body = Vec::new();
        request.as_reader().read_to_end(&mut body).unwrap();
        assert_eq!(body, b"Y2hhbGxlbmdl");
        request
            .respond(tiny_http::Response::from_string(
                r#"{"payload":{"license":"bGljZW5zZQ=="}}"#,
            ))
            .unwrap();
    });
    let settings = Settings {
        allow_http: true,
        ..Default::default()
    };
    let context = network::Context::fixture(&settings, &[("license.example", address)]);
    let config = serde_json::json!({"com.widevine.alpha":{"license":{"server_url":format!("http://license.example:{}/license",address.port()),"req_data":base64::engine::general_purpose::STANDARD.encode(b"{CHA-B64}"),"unwrapper":"json,base64","unwrapper_params":{"path_data":"/payload/license"}}}});
    let entry = Entry {
        id: "personal_fixture".into(),
        name: "Test".into(),
        number: None,
        kind: crate::model::MediaKind::Tv,
        content: "live".into(),
        groups: vec![],
        logo: String::new(),
        url: "https://media.example/live.mpd".into(),
        headers: Headers::new(),
        metadata: Metadata {
            properties: [("inputstream.adaptive.drm".into(), config.to_string())].into(),
            ..Default::default()
        },
        position: 0,
    };
    let policy = drm::policy(&entry, context).unwrap();
    assert_eq!(
        policy
            .drm
            .as_ref()
            .unwrap()
            .license(&policy.context, b"challenge")
            .unwrap(),
        b"license"
    );
    worker.join().unwrap();
}

#[test]
fn revoked_source_metadata_cannot_mint_a_new_playback_context() {
    let f = Fixture::new();
    let source = f.import(&playlist("https://media.example/live.m3u8"), None);
    let prior = f.library.source(&source).unwrap();
    f.library
        .edit(
            &source,
            prior.revision,
            &prior.name,
            false,
            prior.settings.clone(),
        )
        .unwrap();
    assert!(f.library.context(&prior).is_err());
}

#[test]
fn an_interrupted_snapshot_swap_restores_the_registered_copy_on_restart() {
    let f = Fixture::new();
    let source = f.import(&playlist("https://media.example/old.m3u8"), None);
    let rows = f.rows(&source);
    let path = f.library.path(&source).unwrap();
    let backup = f.library.directory.join(format!(".backup-{source}.sqlite"));
    std::fs::copy(&path, &backup).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute(
        "UPDATE metadata SET value='unfinished' WHERE key='source_snapshot'",
        [],
    )
    .unwrap();
    db.execute("DELETE FROM records", []).unwrap();
    drop(db);
    let restored = Library::new(&f.root).unwrap();
    assert_eq!(
        restored
            .catalog(
                &CatalogQuery {
                    source,
                    limit: 200,
                    ..Default::default()
                },
                &[]
            )
            .unwrap(),
        rows
    );
    assert!(!backup.exists());
}

#[test]
fn revocation_cancels_requests_while_waiting_for_headers() {
    use std::io::Read;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (accepted, ready) = std::sync::mpsc::channel();
    let (close, closed) = std::sync::mpsc::channel();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0; 4096];
        assert!(socket.read(&mut request).unwrap() > 0);
        accepted.send(()).unwrap();
        let _ = closed.recv_timeout(Duration::from_secs(5));
    });
    let settings = Settings {
        allow_http: true,
        ..Default::default()
    };
    let context = network::Context::fixture(&settings, &[("media.example", address)]);
    let copy = context.clone();
    let request = thread::spawn(move || {
        copy.request(
            &format!("http://media.example:{}", address.port()),
            &format!("http://media.example:{}/stall", address.port()),
            Resource::Media,
            &Headers::new(),
            reqwest::Method::GET,
            None,
        )
        .is_err()
    });
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    let started = Instant::now();
    context.revoke();
    assert!(request.join().unwrap());
    assert!(started.elapsed() < Duration::from_secs(1));
    close.send(()).unwrap();
    server.join().unwrap();
}

#[test]
fn revocation_interrupts_a_stalled_media_body() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (close, closed) = std::sync::mpsc::channel();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0; 4096];
        assert!(socket.read(&mut request).unwrap() > 0);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10000\r\n\r\nx")
            .unwrap();
        let _ = closed.recv_timeout(Duration::from_secs(5));
    });
    let settings = Settings {
        allow_http: true,
        ..Default::default()
    };
    let context = network::Context::fixture(&settings, &[("media.example", address)]);
    let url = format!("http://media.example:{}/video.mp4", address.port());
    let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let entry = Entry {
        id: String::new(),
        name: "Fixture".into(),
        number: None,
        kind: crate::model::MediaKind::Tv,
        content: "vod".into(),
        groups: vec![],
        logo: String::new(),
        url: url.clone(),
        headers: Headers::new(),
        metadata: Metadata::default(),
        position: 0,
    };
    let policy = drm::policy(&entry, context.clone())
        .unwrap()
        .for_playback(cancelled.clone());
    let mut response = policy
        .media_request(&url, Resource::Media, &Headers::new())
        .unwrap();
    let mut byte = [0];
    response.read_exact(&mut byte).unwrap();
    assert_eq!(&byte, b"x");
    let reader = thread::spawn(move || response.read(&mut byte).is_err());
    let started = Instant::now();
    cancelled.store(true, std::sync::atomic::Ordering::Release);
    assert!(reader.join().unwrap());
    assert!(context.check().is_ok());
    assert!(started.elapsed() < Duration::from_secs(1));
    close.send(()).unwrap();
    server.join().unwrap();
}
