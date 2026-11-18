// SPDX-License-Identifier: GPL-3.0-or-later
//! Negotiate the HLS representation of an Xtream live transport stream.
use super::{network::SessionPolicy, Entry, Headers, Resource};
use anyhow::Result;
use std::io::Read;

fn candidate(entry: &Entry) -> Option<String> {
    if entry.kind != crate::model::MediaKind::Tv || entry.content != "live" {
        return None;
    }
    let mut url = reqwest::Url::parse(&entry.url).ok()?;
    let parts = url.path_segments()?.collect::<Vec<_>>();
    if parts.len() != 4 || parts[0] != "live" || parts[1..3].iter().any(|p| p.is_empty()) {
        return None;
    }
    let id = parts[3].strip_suffix(".ts")?;
    if id.is_empty() || id.len() > 20 || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let file = format!("{id}.m3u8");
    url.path_segments_mut().ok()?.pop().push(&file);
    Some(url.into())
}

pub(super) fn representation(entry: &Entry, policy: &SessionPolicy) -> Result<Option<String>> {
    if policy.drm.is_some()
        || entry
            .metadata
            .properties
            .contains_key("inputstream.adaptive.manifest_type")
    {
        return Ok(None);
    }
    let Some(url) = candidate(entry) else {
        return Ok(None);
    };
    // Keep the original root and its access scope. A redirect cannot inherit
    // credentials unless its origin is already authorized for the manifest role.
    let response = match policy.media_request(&url, Resource::Manifest, &Headers::new()) {
        Ok(response) => response,
        Err(_) => {
            policy.context.check()?;
            return Ok(None);
        }
    };
    if !response.status().is_success() {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    if response.take(4096).read_to_end(&mut bytes).is_err() {
        policy.context.check()?;
        return Ok(None);
    }
    policy.context.check()?;
    let text = String::from_utf8_lossy(&bytes);
    let text = text.trim_start_matches(['\u{feff}', ' ', '\r', '\n', '\t']);
    Ok((text.starts_with("#EXTM3U")
        && (text.contains("#EXT-X-TARGETDURATION:") || text.contains("#EXT-X-STREAM-INF:")))
    .then_some(url))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::personal::{drm, network, tests::Fixture, Settings};
    use std::{thread, time::Duration};

    #[test]
    fn live_hls_negotiation_preserves_storage_headers_and_original_scope() {
        let f = Fixture::new();
        let source = f.import("#EXTM3U\n#EXTINF:-1,Live\n#EXTVLCOPT:http-user-agent=listed-agent\nhttps://media.example/live/user/pass/123.ts?token=synthetic\n",None);
        let id = f.rows(&source)["rows"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let (_, mut entry) = f.library.entry(&id).unwrap();
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        entry.url = format!(
            "http://media.example:{}/live/user/pass/123.ts?token=synthetic",
            address.port()
        );
        let original = entry.url.clone();
        let context = network::Context::fixture(
            &Settings {
                allow_http: true,
                ..Default::default()
            },
            &[("media.example", address)],
        );
        let policy = drm::policy(&entry, context).unwrap();
        let worker = thread::spawn(move || {
            for (status, body) in [
                (
                    200,
                    "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXTINF:6,\nsegment.ts\n",
                ),
                (200, "<html>not a playlist</html>"),
                (403, "#EXTM3U\n#EXT-X-TARGETDURATION:6\n"),
            ] {
                let request = server
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap();
                assert_eq!(request.url(), "/live/user/pass/123.m3u8?token=synthetic");
                assert_eq!(
                    request
                        .headers()
                        .iter()
                        .find(|h| h.field.equiv("User-Agent"))
                        .unwrap()
                        .value
                        .as_str(),
                    "listed-agent"
                );
                request
                    .respond(tiny_http::Response::from_string(body).with_status_code(status))
                    .unwrap();
            }
        });
        assert!(representation(&entry, &policy)
            .unwrap()
            .unwrap()
            .contains("123.m3u8?token=synthetic"));
        assert!(representation(&entry, &policy).unwrap().is_none());
        assert!(representation(&entry, &policy).unwrap().is_none());
        assert_eq!(entry.url, original);
        assert_eq!(policy.root, original);
        for url in [
            "https://media.example/movie/user/pass/123.ts",
            "https://media.example/live/user/pass/name.ts",
            "https://media.example/file.ts",
        ] {
            entry.url = url.into();
            assert!(candidate(&entry).is_none());
        }
        worker.join().unwrap();
    }
}
