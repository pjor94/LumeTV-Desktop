// SPDX-License-Identifier: GPL-3.0-or-later
//! Supplemental public XMLTV feed. It has no access to personal playlist sessions.
use super::{parse_feed, GuideRateLimit, LimitedRead, HOUR};
use crate::{
    public_guide::{ChannelAlias, PublicGuide},
    report::now_ms,
};
use anyhow::{anyhow, bail, Result};
use reqwest::{blocking::Client, header, StatusCode};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

const FEED: &str = "https://epgshare01.online/epgshare01/epg_ripper_IT1.xml.gz";

const CHANNEL_ALIASES: &[ChannelAlias] = &[
    ChannelAlias {
        playlist_id: "eurosport1.it",
        playlist_name: "Eurosport 1",
        feed_id: "eurosport.italia.it",
        feed_name: "Eurosport Italia",
    },
    ChannelAlias {
        playlist_id: "eurosport2.it",
        playlist_name: "Eurosport 2",
        feed_id: "eurosport.2.italia.it",
        feed_name: "Eurosport 2 Italia",
    },
    ChannelAlias {
        playlist_id: "dazneurosport1.it",
        playlist_name: "DAZN Eurosport 1",
        feed_id: "eurosport.italia.it",
        feed_name: "Eurosport Italia",
    },
    ChannelAlias {
        playlist_id: "dazneurosport2.it",
        playlist_name: "DAZN Eurosport 2",
        feed_id: "eurosport.2.italia.it",
        feed_name: "Eurosport 2 Italia",
    },
    ChannelAlias {
        playlist_id: "solocalciosportitaliafonte1.it",
        playlist_name: "Solo Calcio | Sportitalia ( Fonte 1 )",
        feed_id: "solocalcio.it.it",
        feed_name: "Solocalcio",
    },
    ChannelAlias {
        playlist_id: "skyunoplus.it",
        playlist_name: "Sky Uno Plus",
        // IT1 labels both entries "Sky Uno +". The single-dot ID is the one-hour shift.
        feed_id: "sky.uno.+..it",
        feed_name: "Sky Uno +",
    },
];

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct State {
    checked_at: u64,
    attempted_at: u64,
    next_retry_at: u64,
    error: String,
    etag: String,
    modified: String,
}

pub(super) struct EpgShare {
    pub index: PublicGuide,
    state: Mutex<State>,
    loading: AtomicBool,
    cache: PathBuf,
    client: Client,
}

impl EpgShare {
    pub fn new(directory: &Path) -> Result<Arc<Self>> {
        let cache = directory.join("epgshare-guide.json");
        let state = fs::read(&cache)
            .ok()
            .filter(|v| v.len() <= 16 * 1024)
            .and_then(|v| serde_json::from_slice(&v).ok())
            .unwrap_or_default();
        let client = Client::builder()
            .no_proxy()
            .referer(false)
            .user_agent(format!("LumeTV/{}", env!("CARGO_PKG_VERSION")))
            .dns_resolver(Arc::new(crate::network::PublicDns))
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(45))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                let url = attempt.url();
                if attempt.previous().len() < 3
                    && url.scheme() == "https"
                    && url.host_str() == Some("epgshare01.online")
                    && url.port().is_none()
                    && url.username().is_empty()
                    && url.password().is_none()
                {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }))
            .build()
            .map_err(|_| anyhow!("epgshare_client"))?;
        Ok(Arc::new(Self {
            index: PublicGuide::named(directory, "epgshare-guide.sqlite", CHANNEL_ALIASES),
            state: Mutex::new(state),
            loading: AtomicBool::new(false),
            cache,
            client,
        }))
    }

    pub fn status(&self) -> serde_json::Value {
        let info = self.index.info();
        let state = self.state.lock().expect("EPGShare state");
        serde_json::json!({"origin":"epgshare","name":"EPGShare IT1","updated_at":info.updated_at,
            "checked_at":state.checked_at,"loading":self.loading.load(Ordering::Acquire),
            "error":state.error,"next_retry_at":state.next_retry_at,
            "channel_count":info.channels,"programmes":info.programmes})
    }

    fn due(&self, force: bool, at: u64) -> bool {
        let ready = self.index.ready();
        let state = self.state.lock().expect("EPGShare state");
        at >= state.next_retry_at
            && (state.attempted_at == 0 || at.saturating_sub(state.attempted_at) >= 30_000)
            && (force || !ready || at.saturating_sub(state.checked_at) >= 4 * HOUR)
    }

    pub fn refresh(self: &Arc<Self>, force: bool) {
        let at = now_ms();
        if !self.due(force, at) || self.loading.swap(true, Ordering::AcqRel) {
            return;
        }
        self.state.lock().expect("EPGShare state").attempted_at = at;
        self.save();
        let service = self.clone();
        std::thread::spawn(move || {
            service.attempt(FEED, at);
            service.loading.store(false, Ordering::Release);
        });
    }

    fn attempt(&self, endpoint: &str, at: u64) {
        if let Err(error) = self.fetch_from(endpoint, at) {
            let seconds = error
                .downcast_ref::<GuideRateLimit>()
                .map_or(300, |e| e.seconds);
            let mut state = self.state.lock().expect("EPGShare state");
            state.next_retry_at = now_ms().max(at).saturating_add(seconds * 1000);
            state.error = if error.is::<GuideRateLimit>() {
                "EPGShare richiede una pausa. La copia salvata resta disponibile."
            } else {
                "EPGShare non è raggiungibile. La copia salvata resta disponibile."
            }
            .into();
        }
        self.save();
    }

    fn fetch_from(&self, endpoint: &str, at: u64) -> Result<()> {
        let previous = self.state.lock().expect("EPGShare state").clone();
        let mut request = self.client.get(endpoint);
        // Re-index at least twice a day so the retained 48-hour window moves
        // forward even if a server keeps returning an unchanged document.
        let conditional =
            self.index.ready() && at.saturating_sub(self.index.info().updated_at) < 12 * HOUR;
        if conditional {
            if !previous.etag.is_empty() {
                request = request.header(header::IF_NONE_MATCH, &previous.etag);
            }
            if !previous.modified.is_empty() {
                request = request.header(header::IF_MODIFIED_SINCE, &previous.modified);
            }
        }
        let response = request.send().map_err(|_| anyhow!("epgshare_network"))?;
        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            let seconds = response
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(3600)
                .clamp(60, 86_400);
            return Err(GuideRateLimit { seconds }.into());
        }
        if response.status() == StatusCode::NOT_MODIFIED {
            if !conditional || previous.etag.is_empty() && previous.modified.is_empty() {
                bail!("epgshare_empty_304");
            }
            let mut state = self.state.lock().expect("EPGShare state");
            state.checked_at = at;
            state.error.clear();
            state.next_retry_at = 0;
            return Ok(());
        }
        if !response.status().is_success()
            || response
                .content_length()
                .is_some_and(|n| n > 32 * 1024 * 1024)
        {
            bail!("epgshare_response");
        }
        let validator = |key| {
            response
                .headers()
                .get(key)
                .and_then(|v| v.to_str().ok())
                .filter(|v| v.len() <= 1024 && !v.chars().any(char::is_control))
                .unwrap_or("")
                .to_owned()
        };
        let etag = validator(header::ETAG);
        let modified = validator(header::LAST_MODIFIED);
        let mut compressed = Vec::new();
        response
            .take(32 * 1024 * 1024 + 1)
            .read_to_end(&mut compressed)
            .map_err(|_| anyhow!("epgshare_download"))?;
        if compressed.len() > 32 * 1024 * 1024 {
            bail!("epgshare_size_limit");
        }
        let decoder = flate2::read::GzDecoder::new(compressed.as_slice());
        self.publish_xml(
            BufReader::new(LimitedRead::new(decoder, 350 * 1024 * 1024)),
            at,
            at,
        )?;
        let mut state = self.state.lock().expect("EPGShare state");
        state.checked_at = at;
        state.next_retry_at = 0;
        state.error.clear();
        state.etag = etag;
        state.modified = modified;
        Ok(())
    }

    fn publish_xml<R: BufRead>(&self, input: R, downloaded: u64, at: u64) -> Result<()> {
        let mut writer = self.index.begin(downloaded)?;
        parse_feed(input, at, &[], Some(&mut writer))?;
        self.index.publish(writer)
    }

    pub fn import_xml<R: BufRead>(&self, input: R, downloaded: u64) -> Result<serde_json::Value> {
        self.publish_xml(
            BufReader::new(LimitedRead::new(input, 350 * 1024 * 1024)),
            downloaded,
            now_ms(),
        )?;
        *self.state.lock().expect("EPGShare state") = State {
            checked_at: downloaded,
            ..Default::default()
        };
        self.save();
        Ok(self.status())
    }

    fn save(&self) {
        let state = self.state.lock().expect("EPGShare state");
        if let Ok(bytes) = serde_json::to_vec(&*state) {
            let temporary = self.cache.with_extension("json.tmp");
            if fs::write(&temporary, bytes).is_ok() {
                let _ = fs::rename(&temporary, &self.cache);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::public_guide::Identity;
    use std::io::{Cursor, Write};

    struct Fixture {
        root: PathBuf,
        service: Arc<EpgShare>,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("lume-epgshare-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&root).unwrap();
            Self {
                service: EpgShare::new(&root).unwrap(),
                root,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn xml() -> String {
        let now = chrono::Utc::now();
        let start = (now - chrono::Duration::minutes(10)).format("%Y%m%d%H%M%S +0000");
        let stop = (now + chrono::Duration::minutes(50)).format("%Y%m%d%H%M%S +0000");
        let later = (now + chrono::Duration::hours(13)).format("%Y%m%d%H%M%S +0000");
        let later_stop = (now + chrono::Duration::hours(14)).format("%Y%m%d%H%M%S +0000");
        format!(
            r#"<tv><channel id="Sky.Uno.it"><display-name>Sky Uno</display-name><icon src="https://guidatv.sky.it/logo/uno.png"/></channel><programme channel="Sky.Uno.it" start="{start}" stop="{stop}"><title>Programma Sky di prova</title></programme><programme channel="Sky.Uno.it" start="{later}" stop="{later_stop}"><title>Programma successivo</title></programme></tv>"#
        )
    }

    #[test]
    fn known_italian_names_use_the_correct_alias_in_cards_and_guide() {
        let f = Fixture::new();
        let at = now_ms();
        let mut writer = f.service.index.begin(at).unwrap();
        for (id, name, title) in [
            (
                "Eurosport.Italia.it",
                "Eurosport Italia",
                "Evento Eurosport 1",
            ),
            (
                "Eurosport.2.Italia.it",
                "Eurosport 2 Italia",
                "Evento Eurosport 2",
            ),
            ("Solocalcio.it.it", "Solocalcio", "Evento Solo Calcio"),
            ("Sky.Uno.+..it", "Sky Uno +", "Programma Sky Uno Plus"),
            ("Sky.Uno.+.it", "Sky Uno +", "Programma in differita"),
        ] {
            writer.channel(id, &[name.into()], "").unwrap();
            writer
                .programme(
                    id,
                    &crate::guide::Programme {
                        title: title.into(),
                        start: at - 60_000,
                        end: at + 60_000,
                        ..Default::default()
                    },
                )
                .unwrap();
        }
        f.service.index.publish(writer).unwrap();
        let index = &EpgShare::new(&f.root).unwrap().index;
        let identity = |id: &str, name: &str| Identity {
            xml_id: id.into(),
            names: vec![name.into()],
        };
        for (id, name, title) in [
            ("Eurosport1.it", "Eurosport 1 HD", "Evento Eurosport 1"),
            (
                "DaznEurosport2.it",
                "DAZN Eurosport 2",
                "Evento Eurosport 2",
            ),
            (
                "SoloCalcioSportitaliaFonte1.it",
                "Solo Calcio | Sportitalia ( Fonte 1 )",
                "Evento Solo Calcio",
            ),
            ("SkyUnoPlus.it", "Sky Uno Plus", "Programma Sky Uno Plus"),
            ("Sky.Uno.+.it", "Sky Uno +", "Programma in differita"),
        ] {
            let station = identity(id, name);
            let card = index
                .lookup(std::slice::from_ref(&station), at)
                .pop()
                .unwrap();
            assert_eq!(card.data.unwrap().now.unwrap().title, title);
            let (events, _) = index
                .window(&station, at - 60_000, at + 60_000, "", 0)
                .unwrap();
            assert_eq!(events[0].title, title);
        }
        let excluded = [
            identity("Eurosport1.fr", "Eurosport 1"),
            identity("Eurosport1.it", "Eurosport 360 1"),
            identity("SkyUnoPlus.it", "Sky Uno +1"),
            identity("", "Sky Uno +"),
        ];
        assert!(index
            .lookup(&excluded, at)
            .iter()
            .all(|probe| probe.data.is_none()));
    }

    #[test]
    fn public_gzip_uses_validators_and_keeps_download_time_after_304() {
        let f = Fixture::new();
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let endpoint = format!(
            "http://{}/guide.xml.gz",
            server.server_addr().to_ip().unwrap()
        );
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(xml().as_bytes()).unwrap();
        let body = gzip.finish().unwrap();
        let worker = std::thread::spawn(move || {
            for i in 0..3 {
                let request = server
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap();
                let header = |key| {
                    request
                        .headers()
                        .iter()
                        .find(|h| h.field.equiv(key))
                        .map(|h| h.value.as_str())
                };
                assert_eq!(
                    header("User-Agent"),
                    Some(concat!("LumeTV/", env!("CARGO_PKG_VERSION")))
                );
                assert_eq!(header("Authorization"), None);
                assert_eq!(header("Cookie"), None);
                assert_eq!(header("Referer"), None);
                if i == 1 {
                    assert_eq!(header("If-None-Match"), Some("\"public-guide-v1\""));
                    assert_eq!(
                        header("If-Modified-Since"),
                        Some("Wed, 18 Nov 2026 05:38:00 GMT")
                    );
                    request.respond(tiny_http::Response::empty(304)).unwrap();
                } else {
                    assert_eq!(header("If-None-Match"), None);
                    request
                        .respond(
                            tiny_http::Response::from_data(body.clone())
                                .with_header(
                                    tiny_http::Header::from_bytes("ETag", "\"public-guide-v1\"")
                                        .unwrap(),
                                )
                                .with_header(
                                    tiny_http::Header::from_bytes(
                                        "Last-Modified",
                                        "Wed, 18 Nov 2026 05:38:00 GMT",
                                    )
                                    .unwrap(),
                                ),
                        )
                        .unwrap();
                }
            }
        });
        let at = now_ms();
        f.service.attempt(&endpoint, at);
        assert_eq!(f.service.index.info().channels, 1);
        let identity = Identity {
            xml_id: "Sky.Uno.it".into(),
            names: vec!["Sky Uno".into()],
        };
        assert_eq!(
            f.service.index.lookup(&[identity], at)[0]
                .data
                .as_ref()
                .unwrap()
                .now
                .as_ref()
                .unwrap()
                .title,
            "Programma Sky di prova"
        );
        assert!(!f.service.due(false, at + HOUR));
        f.service.attempt(&endpoint, at + HOUR);
        assert_eq!(f.service.index.info().updated_at, at);
        assert_eq!(f.service.status()["checked_at"], at + HOUR);
        f.service.fetch_from(&endpoint, at + 13 * HOUR).unwrap();
        assert_eq!(f.service.index.info().updated_at, at + 13 * HOUR);
        worker.join().unwrap();
        assert!(EpgShare::new(&f.root).unwrap().index.ready());
    }

    #[test]
    fn failures_and_rate_limits_keep_last_valid_index_and_persist_backoff() {
        let f = Fixture::new();
        let at = now_ms();
        f.service.import_xml(Cursor::new(xml()), at).unwrap();
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/guide", server.server_addr().to_ip().unwrap());
        let worker = std::thread::spawn(move || {
            let request = server
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap();
            request
                .respond(
                    tiny_http::Response::empty(429)
                        .with_header(tiny_http::Header::from_bytes("Retry-After", "120").unwrap()),
                )
                .unwrap();
            let request = server
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap();
            request
                .respond(tiny_http::Response::from_string("<html>not gzip</html>"))
                .unwrap();
        });
        f.service.attempt(&endpoint, at);
        assert!(f.service.status()["next_retry_at"].as_u64().unwrap() >= at + 120_000);
        assert!(!f.service.due(true, at + 60_000));
        assert!(EpgShare::new(&f.root).unwrap().status()["error"]
            .as_str()
            .unwrap()
            .contains("pausa"));
        f.service.attempt(&endpoint, at + 120_001);
        worker.join().unwrap();
        assert_eq!(f.service.index.info().updated_at, at);
        assert_eq!(f.service.index.info().programmes, 2);
        assert!(f
            .service
            .import_xml(Cursor::new("<tv><channel id='broken'>"), at + 1)
            .is_err());
        assert_eq!(f.service.index.info().updated_at, at);
        assert!(fs::read_dir(&f.root).unwrap().flatten().all(|f| !f
            .file_name()
            .to_string_lossy()
            .starts_with(".public-guide-")));
    }
}
