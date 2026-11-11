// SPDX-License-Identifier: GPL-3.0-or-later
//! Authenticated loopback network adapter. It forwards opaque EME challenges and
//! license replies; decryption and key handling remain entirely in the browser CDM.
use crate::{
    model::{Channel, ResolvedStream},
    report::{now_ms, sanitize_event},
};
use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::{blocking::Client, Url};
use serde_json::{json, Value};
use std::{
    io::Read,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

const MAX_BODY: usize = 2 * 1024 * 1024;
const MAX_MEDIA: usize = 32 * 1024 * 1024;
type RelayResponse = (u16, String, String, Vec<u8>, Vec<(String, String)>);

pub struct State {
    pub events: Mutex<Vec<Value>>,
    pub complete: AtomicBool,
    pub stop: AtomicBool,
    pub license_success: AtomicU64,
    pub media_success: AtomicU64,
    pub network_failures: AtomicU64,
    pub transferred_bytes: AtomicU64,
    document_requests: AtomicU64,
    script_requests: AtomicU64,
    session_requests: AtomicU64,
    token: String,
    origin: String,
    channel: Channel,
    stream: ResolvedStream,
    duration: u64,
    drm_setup_timeout_seconds: AtomicU64,
    mode: String,
    media_client: Client,
    license_client: Client,
    active: AtomicUsize,
}

pub struct HarnessServer {
    pub state: Arc<State>,
    pub origin: String,
    pub launch_url: String,
    handle: Option<thread::JoinHandle<()>>,
}

impl HarnessServer {
    pub fn start(
        channel: Channel,
        stream: ResolvedStream,
        duration: u64,
        mode: &str,
    ) -> Result<Self> {
        let server = Server::http("127.0.0.1:0")
            .map_err(|_| anyhow!("Cannot bind loopback player server"))?;
        let address = server
            .server_addr()
            .to_ip()
            .context("Missing loopback address")?;
        let origin = format!("http://{address}");
        let token = uuid::Uuid::new_v4().to_string();
        let media_stream = stream.clone();
        let radio = channel.kind == crate::model::MediaKind::Radio;
        let media_client = Client::builder()
            .no_proxy()
            .dns_resolver(Arc::new(crate::network::PublicDns))
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .referer(false)
            .redirect(reqwest::redirect::Policy::custom(move |attempt| {
                if attempt.previous().len() >= 8
                    || !(if radio {
                        crate::radio::allowed_url(attempt.url())
                    } else {
                        allowed_media(attempt.url(), &media_stream)
                    })
                {
                    attempt.stop()
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .map_err(|_| anyhow!("Cannot initialize network adapter"))?;
        // Never forward provider-specific authorization headers to a redirected
        // license host. A changed endpoint must be returned by the resolver.
        let license_client = Client::builder()
            .no_proxy()
            .dns_resolver(Arc::new(crate::network::PublicDns))
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .referer(false)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| anyhow!("Cannot initialize license adapter"))?;
        let state = Arc::new(State {
            events: Mutex::new(vec![]),
            complete: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            license_success: AtomicU64::new(0),
            media_success: AtomicU64::new(0),
            network_failures: AtomicU64::new(0),
            transferred_bytes: AtomicU64::new(0),
            document_requests: AtomicU64::new(0),
            script_requests: AtomicU64::new(0),
            session_requests: AtomicU64::new(0),
            token: token.clone(),
            origin: origin.clone(),
            channel,
            stream,
            duration,
            drm_setup_timeout_seconds: AtomicU64::new(0),
            mode: mode.into(),
            media_client,
            license_client,
            active: AtomicUsize::new(0),
        });
        let shared = state.clone();
        let handle = thread::spawn(move || {
            while !shared.stop.load(Ordering::Relaxed) {
                if let Ok(Some(request)) = server.recv_timeout(Duration::from_millis(200)) {
                    if shared.active.fetch_add(1, Ordering::Relaxed) >= 16 {
                        shared.active.fetch_sub(1, Ordering::Relaxed);
                        respond_json(request, 503, json!({"error":"busy"}));
                        continue;
                    }
                    let state = shared.clone();
                    thread::spawn(move || {
                        handle_request(request, &state);
                        state.active.fetch_sub(1, Ordering::Relaxed);
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

    pub fn set_drm_setup_timeout(&self, seconds: u64) {
        self.state
            .drm_setup_timeout_seconds
            .store(seconds.min(300), Ordering::Relaxed);
    }

    pub fn network_summary(&self) -> Value {
        json!({"license_http_successes":self.state.license_success.load(Ordering::Relaxed),"media_http_successes":self.state.media_success.load(Ordering::Relaxed),"failed_requests":self.state.network_failures.load(Ordering::Relaxed),"transferred_bytes":self.state.transferred_bytes.load(Ordering::Relaxed),"document_requests":self.state.document_requests.load(Ordering::Relaxed),"script_requests":self.state.script_requests.load(Ordering::Relaxed),"session_requests":self.state.session_requests.load(Ordering::Relaxed)})
    }
}

impl Drop for HarnessServer {
    fn drop(&mut self) {
        self.state.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
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

fn handle_request(mut request: Request, state: &State) {
    let path = request.url().split('?').next().unwrap_or("").to_owned();
    let expected_host = state.origin.trim_start_matches("http://");
    if header(&request, "Host") != Some(expected_host) {
        respond_json(request, 403, json!({"error":"invalid_host"}));
        return;
    }
    if path.starts_with("/api/")
        && (header(&request, "X-Lume-Token") != Some(state.token.as_str())
            || header(&request, "Origin").is_some_and(|o| o != state.origin))
    {
        respond_json(request, 403, json!({"error":"unauthorized"}));
        return;
    }
    if request.method() == &Method::Get {
        let counter = match path.as_str() {
            "/" => Some(&state.document_requests),
            "/player.js" | "/vendor/shaka-player.compiled.js" => Some(&state.script_requests),
            "/api/session" => Some(&state.session_requests),
            _ => None,
        };
        if let Some(counter) = counter {
            counter.fetch_add(1, Ordering::Relaxed);
        }
        match path.as_str() {
            "/" => respond(
                request,
                200,
                "text/html; charset=utf-8",
                include_bytes!("../web/index.html").to_vec(),
                vec![],
            ),
            "/player.js" => respond(
                request,
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../web/player.js").to_vec(),
                vec![],
            ),
            "/vendor/shaka-player.compiled.js" => respond(
                request,
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../vendor/shaka/dist/shaka-player.compiled.js").to_vec(),
                vec![],
            ),
            "/api/session" => respond_json(
                request,
                200,
                json!({"channel":state.channel,"stream":{"url":state.stream.url,"mime_type":state.stream.mime_type,"drm":state.stream.drm.as_ref().map(|_|json!({"key_system":"com.widevine.alpha"}))},"mode":state.mode,"duration_seconds":state.duration,"drm_setup_timeout_seconds":state.drm_setup_timeout_seconds.load(Ordering::Relaxed)}),
            ),
            "/favicon.ico" => respond(request, 204, "image/x-icon", vec![], vec![]),
            _ => respond_json(request, 404, json!({"error":"not_found"})),
        }
        return;
    }
    if request.method() != &Method::Post || !["/api/event", "/api/fetch"].contains(&path.as_str()) {
        respond_json(request, 405, json!({"error":"method_not_allowed"}));
        return;
    }
    let mut body = Vec::new();
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
    if path == "/api/event" {
        let Some(event) = sanitize_event(&value) else {
            respond_json(request, 400, json!({"error":"invalid_event"}));
            return;
        };
        let done = event["kind"] == "complete";
        if let Ok(mut events) = state.events.lock() {
            if events.len() < 4096 {
                events.push(event);
            }
        }
        if done {
            state.complete.store(true, Ordering::Relaxed);
        }
        respond_json(request, 200, json!({"ok":true}));
    } else {
        relay(request, state, &value);
    }
}

fn public_https(url: &Url) -> bool {
    crate::network::public_host(url)
        && url.scheme() == "https"
        && url.port().is_none_or(|p| p == 443)
}

pub fn allowed_media(url: &Url, stream: &ResolvedStream) -> bool {
    if !crate::network::public_host(url) {
        return false;
    }
    let Ok(initial) = Url::parse(&stream.url) else {
        return false;
    };
    // A small number of catalogue radios still use HTTP on a custom port.
    // Confine that transport to the exact origin declared by their resolver.
    if !public_https(url) {
        return initial.scheme() == "http" && initial.origin() == url.origin();
    }
    let host = url.host_str().unwrap_or_default();
    // The prototype is restricted to broadcaster/CDN hosts, not a general proxy.
    host == initial.host_str().unwrap_or_default()
        || [
            "rai.it",
            "raiplay.it",
            "akamaized.net",
            "akamaihd.net",
            "mediaset.net",
            "mediaset.it",
            "theplatform.eu",
            "la7.it",
            "la7play.it",
            "la7cloud.it",
            "sky.it",
            "skycdn.it",
            "skycdp.com",
            "discoverycdn.net",
            "discoveryplus.com",
            "dplay.com",
            "cloudfront.net",
            "msvdn.net",
            "streamlock.net",
            "b-cdn.net",
            "morescreens.com",
            "xdevel.com",
            "netrw.it",
            "multistream.it",
            "bitonlive.net",
            "rdstv.radio",
            "premio.link",
            "getaj.net",
            "bloomberg.com",
            "wurl.com",
            "amagi.tv",
        ]
        .iter()
        .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
}

fn relay(request: Request, state: &State, input: &Value) {
    let license = input["kind"] == "license";
    let target = if license {
        state.stream.drm.as_ref().map(|d| d.license_url.as_str())
    } else if input["kind"] == "media" {
        input["url"].as_str()
    } else {
        None
    };
    let Some(target) = target else {
        respond_json(request, 400, json!({"error":"invalid_target"}));
        return;
    };
    let Ok(url) = Url::parse(target) else {
        respond_json(request, 400, json!({"error":"invalid_target"}));
        return;
    };
    if if license {
        !public_https(&url)
    } else {
        !(if state.channel.kind == crate::model::MediaKind::Radio {
            crate::radio::allowed_url(&url)
        } else {
            allowed_media(&url, &state.stream)
        })
    } {
        state.network_failures.fetch_add(1, Ordering::Relaxed);
        record_network(state, license, 0, "blocked_host");
        respond_json(request, 403, json!({"error":"blocked_host"}));
        return;
    }
    let method = input["method"]
        .as_str()
        .unwrap_or(if license { "POST" } else { "GET" });
    if (!license && method != "GET" && method != "HEAD") || (license && method != "POST") {
        respond_json(request, 405, json!({"error":"method_not_allowed"}));
        return;
    }
    let client = if license {
        &state.license_client
    } else {
        &state.media_client
    };
    let mut builder = client.request(
        if method == "POST" {
            reqwest::Method::POST
        } else if method == "HEAD" {
            reqwest::Method::HEAD
        } else {
            reqwest::Method::GET
        },
        url,
    );
    let headers = if license {
        &state.stream.drm.as_ref().unwrap().headers
    } else {
        &state.stream.headers
    };
    for (key, value) in headers {
        builder = builder.header(key, value);
    }
    if let Some(headers) = input["headers"].as_object() {
        for name in [
            "Range",
            "range",
            "Content-Type",
            "content-type",
            "Accept",
            "accept",
        ] {
            if let Some(value) = headers.get(name).and_then(Value::as_str) {
                let provider_has_header = if license {
                    state
                        .stream
                        .drm
                        .as_ref()
                        .unwrap()
                        .headers
                        .keys()
                        .any(|h| h.eq_ignore_ascii_case(name))
                } else {
                    state
                        .stream
                        .headers
                        .keys()
                        .any(|h| h.eq_ignore_ascii_case(name))
                };
                if !provider_has_header {
                    builder = builder.header(name, value);
                }
            }
        }
    }
    if license {
        let body = match input["body_base64"].as_str().map(|b| STANDARD.decode(b)) {
            Some(Ok(b)) if b.len() <= MAX_BODY => b,
            _ => {
                respond_json(request, 400, json!({"error":"invalid_license_challenge"}));
                return;
            }
        };
        builder = builder.body(body);
    }
    let result = (|| -> Result<RelayResponse> {
        let response = builder.send().map_err(|_| anyhow!("upstream_network"))?;
        let status = response.status().as_u16();
        record_network(state, license, status, "response");
        if !response.status().is_success() {
            return Ok((status, String::new(), String::new(), vec![], vec![]));
        }
        let mime = response
            .headers()
            .get("Content-Type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let final_url = if license {
            String::new()
        } else {
            response.url().to_string()
        };
        let mut extra = vec![];
        for name in ["content-range", "accept-ranges"] {
            if let Some(value) = response.headers().get(name).and_then(|v| v.to_str().ok()) {
                extra.push((name.into(), value.into()));
            }
        }
        let limit = if license { MAX_BODY } else { MAX_MEDIA };
        let mut bytes = vec![];
        response
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| anyhow!("upstream_read"))?;
        if bytes.len() > limit {
            return Err(anyhow!("upstream_too_large"));
        }
        Ok((status, mime, final_url, bytes, extra))
    })();
    match result {
        Ok((status, mime, final_url, bytes, mut extra)) if (200..300).contains(&status) => {
            if license {
                state.license_success.fetch_add(1, Ordering::Relaxed);
            } else {
                state.media_success.fetch_add(1, Ordering::Relaxed);
                extra.push(("X-Lume-Final-Url".into(), final_url));
            }
            state
                .transferred_bytes
                .fetch_add(bytes.len() as u64, Ordering::Relaxed);
            respond(request, status, &mime, bytes, extra);
        }
        Ok((status, _, _, _, _)) => {
            state.network_failures.fetch_add(1, Ordering::Relaxed);
            respond_json(
                request,
                502,
                json!({"error":"upstream_http","status":status}),
            );
        }
        Err(_) => {
            state.network_failures.fetch_add(1, Ordering::Relaxed);
            record_network(state, license, 0, "network_error");
            respond_json(request, 502, json!({"error":"upstream_network"}));
        }
    }
}

fn record_network(state: &State, license: bool, status: u16, outcome: &str) {
    if let Ok(mut events) = state.events.lock() {
        if events.len() < 4096 {
            events.push(json!({"kind":"network","received_unix_ms":now_ms(),"request_type":if license {"license"} else {"media"},"status":status,"outcome":outcome}));
        }
    }
}

fn respond_json(request: Request, status: u16, value: Value) {
    respond(
        request,
        status,
        "application/json",
        serde_json::to_vec(&value).unwrap_or_default(),
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
    for (key,value) in [("Content-Type",mime),("Cache-Control","no-store"),("Referrer-Policy","no-referrer"),("X-Content-Type-Options","nosniff"),("Content-Security-Policy","default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self' https:; media-src 'self' blob: https:; worker-src 'self' blob:; object-src 'none'; frame-ancestors 'none'")] {
        if let Ok(header)=Header::from_bytes(key,value) {response.add_header(header);}
    }
    for (key, value) in extra {
        if let Ok(header) = Header::from_bytes(key, value) {
            response.add_header(header);
        }
    }
    let _ = request.respond(response);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_http_radio_stays_on_its_declared_origin_and_https_streams_cannot_downgrade() {
        let stream = ResolvedStream {
            url: "http://radio.example:8080/live.m3u8".into(),
            mime_type: None,
            headers: Default::default(),
            drm: None,
        };
        assert!(allowed_media(
            &Url::parse("http://radio.example:8080/segment.ts").unwrap(),
            &stream
        ));
        for target in [
            "http://radio.example:8081/segment.ts",
            "http://other.example:8080/live",
            "http://localhost:8080/live",
            "http://127.0.0.1:8080/live",
        ] {
            assert!(!allowed_media(&Url::parse(target).unwrap(), &stream));
        }
        let secure = ResolvedStream {
            url: "https://radio.example/live.m3u8".into(),
            ..stream
        };
        assert!(!allowed_media(
            &Url::parse("http://radio.example/live.m3u8").unwrap(),
            &secure
        ));
    }
    #[test]
    fn relay_rejects_local_and_non_https_targets() {
        for s in [
            "http://example.com/",
            "https://127.0.0.1/",
            "https://[::1]/",
            "https://localhost/",
            "https://machine.local/",
            "https://user:pass@example.com/",
            "https://example.com:8443/",
        ] {
            assert!(!public_https(&Url::parse(s).unwrap()), "{s}");
        }
    }
    #[test]
    fn cdn_suffix_requires_domain_boundary() {
        let s = ResolvedStream {
            url: "https://initial.example/live.mpd".into(),
            mime_type: None,
            headers: Default::default(),
            drm: None,
        };
        assert!(allowed_media(
            &Url::parse("https://a.akamaized.net/a").unwrap(),
            &s
        ));
        assert!(!allowed_media(
            &Url::parse("https://evilakamaized.net/a").unwrap(),
            &s
        ));
    }
}
