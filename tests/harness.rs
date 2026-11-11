// SPDX-License-Identifier: GPL-3.0-or-later
//! Transport contract tests for the real loopback API. Every outbound target is
//! synthetic and every fetch below must be rejected before an upstream request.

use anyhow::Result;
use lume_player::{
    model::{Channel, DrmSpec, ResolvedStream},
    server::HarnessServer,
};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::{collections::BTreeMap, time::Duration};

const PRIVATE: &str = "synthetic-provider-secret-never-return-to-browser";

struct Fixture {
    server: HarnessServer,
    client: Client,
    token: String,
}

impl Fixture {
    fn new() -> Result<Self> {
        let channel = Channel {
            kind: Default::default(),
            headers: Default::default(),
            id: "fixture".into(),
            name: "Synthetic fixture".into(),
            provider: "fixture".into(),
            key: "fixture".into(),
        };
        let stream = ResolvedStream {
            url: "https://example.com/live.mpd".into(),
            mime_type: Some("application/dash+xml".into()),
            headers: BTreeMap::from([("X-Stream-Secret".into(), PRIVATE.into())]),
            drm: Some(DrmSpec {
                license_url: format!("https://example.com/widevine?token={PRIVATE}"),
                headers: BTreeMap::from([
                    ("Content-Type".into(), "application/octet-stream".into()),
                    ("preAuthorization".into(), PRIVATE.into()),
                ]),
            }),
        };
        let server = HarnessServer::start(channel, stream, 2, "relay")?;
        let token = server
            .launch_url
            .split_once('#')
            .expect("launch URL must carry the local session token")
            .1
            .to_owned();
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()?;
        Ok(Self {
            server,
            client,
            token,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.server.origin)
    }

    fn authorized_post(&self, path: &str, body: &Value) -> Result<reqwest::blocking::Response> {
        Ok(self
            .client
            .post(self.url(path))
            .header("X-Lume-Token", &self.token)
            .header("Origin", &self.server.origin)
            .json(body)
            .send()?)
    }
}

#[test]
fn loopback_session_requires_token_host_and_origin_and_keeps_provider_secrets_private() -> Result<()>
{
    let fixture = Fixture::new()?;
    fixture.server.set_drm_setup_timeout(120);

    let unauthenticated = fixture.client.get(fixture.url("/api/session")).send()?;
    assert_eq!(unauthenticated.status().as_u16(), 403);
    assert_eq!(unauthenticated.json::<Value>()?["error"], "unauthorized");

    let wrong_token = fixture
        .client
        .get(fixture.url("/api/session"))
        .header("X-Lume-Token", "incorrect-session-token")
        .send()?;
    assert_eq!(wrong_token.status().as_u16(), 403);

    let wrong_host = fixture
        .client
        .get(fixture.url("/api/session"))
        .header("Host", "untrusted.example")
        .header("X-Lume-Token", &fixture.token)
        .send()?;
    assert_eq!(wrong_host.status().as_u16(), 403);
    assert_eq!(wrong_host.json::<Value>()?["error"], "invalid_host");

    let wrong_origin = fixture
        .client
        .get(fixture.url("/api/session"))
        .header("X-Lume-Token", &fixture.token)
        .header("Origin", "https://untrusted.example")
        .send()?;
    assert_eq!(wrong_origin.status().as_u16(), 403);
    assert_eq!(wrong_origin.json::<Value>()?["error"], "unauthorized");

    let response = fixture
        .client
        .get(fixture.url("/api/session"))
        .header("X-Lume-Token", &fixture.token)
        .header("Origin", &fixture.server.origin)
        .send()?;
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let session: Value = response.json()?;
    assert_eq!(session["channel"]["id"], "fixture");
    assert_eq!(session["stream"]["url"], "https://example.com/live.mpd");
    assert_eq!(session["stream"]["drm"]["key_system"], "com.widevine.alpha");
    assert_eq!(session["drm_setup_timeout_seconds"], 120);
    let serialized = session.to_string();
    for forbidden in [
        PRIVATE,
        "license_url",
        "preAuthorization",
        "X-Stream-Secret",
    ] {
        assert!(!serialized.contains(forbidden));
    }

    let sample = json!({
        "kind": "sample",
        "current_time": 1.25,
        "video_frames": 32,
        "width": 1280,
        "height": 720,
        "ready_state": 4,
        "paused": false,
        "key_statuses": ["usable", PRIVATE],
        "url": format!("https://example.com/?token={PRIVATE}"),
        "headers": {"Authorization": PRIVATE},
        "message": PRIVATE,
        "reason": PRIVATE,
        "arbitrary": PRIVATE,
    });
    let event_response = fixture.authorized_post("/api/event", &sample)?;
    assert_eq!(event_response.status().as_u16(), 200);
    let events = fixture.server.state.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["kind"], "sample");
    assert_eq!(events[0]["current_time"], 1.25);
    assert_eq!(events[0]["video_frames"], 32);
    assert_eq!(events[0]["key_statuses"], json!(["usable"]));
    assert!(events[0]["received_unix_ms"].as_u64().unwrap_or(0) > 0);
    assert!(!serde_json::to_string(&*events)?.contains(PRIVATE));
    assert!(events[0].get("url").is_none());
    assert!(events[0].get("headers").is_none());
    assert!(events[0].get("arbitrary").is_none());
    Ok(())
}

#[test]
fn loopback_rejects_unauthorized_or_invalid_fetches_without_upstream_traffic() -> Result<()> {
    let fixture = Fixture::new()?;

    // Although this target would be permitted for an authenticated media request,
    // authentication must fail before the adapter can make any upstream request.
    let unauthorized = fixture
        .client
        .post(fixture.url("/api/fetch"))
        .json(&json!({"kind":"media", "url":"https://example.com/live.mpd"}))
        .send()?;
    assert_eq!(unauthorized.status().as_u16(), 403);
    assert_eq!(unauthorized.json::<Value>()?["error"], "unauthorized");

    // A syntactically valid license endpoint is configured in the fixture, but a
    // missing/invalid challenge must be rejected without contacting that endpoint.
    for body in [
        json!({"kind":"license", "method":"POST"}),
        json!({"kind":"license", "method":"POST", "body_base64":"%%%invalid%%%"}),
    ] {
        let invalid_challenge = fixture.authorized_post("/api/fetch", &body)?;
        assert_eq!(invalid_challenge.status().as_u16(), 400);
        assert_eq!(
            invalid_challenge.json::<Value>()?["error"],
            "invalid_license_challenge"
        );
    }

    let unsupported_host = fixture.authorized_post(
        "/api/fetch",
        &json!({"kind":"media", "url":"https://untrusted.example/segment.m4s"}),
    )?;
    assert_eq!(unsupported_host.status().as_u16(), 403);
    assert_eq!(unsupported_host.json::<Value>()?["error"], "blocked_host");

    let summary = fixture.server.network_summary();
    assert_eq!(summary["license_http_successes"], 0);
    assert_eq!(summary["media_http_successes"], 0);
    assert_eq!(summary["transferred_bytes"], 0);
    assert_eq!(summary["failed_requests"], 1);
    let events = fixture.server.state.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["kind"], "network");
    assert_eq!(events[0]["outcome"], "blocked_host");
    assert_eq!(events[0]["status"], 0);
    assert!(!serde_json::to_string(&*events)?.contains(PRIVATE));
    Ok(())
}
