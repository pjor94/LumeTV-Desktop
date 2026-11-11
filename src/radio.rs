// SPDX-License-Identifier: GPL-3.0-or-later
//! WorldRadio importer, adapted from the Android RadioCatalog by Lume TV.
use crate::{catalog::CatalogChannel, model::MediaKind, report::now_ms};
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::{blocking::Client, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

pub const BUNDLED: &str = include_str!("../resources/radios.json");
pub const SOURCE: &str = "https://www.worldtvlive.eu/world/files/worldradio/IT.bin";
const LIMIT: usize = 4 * 1024 * 1024;
const DAY: u64 = 86_400_000;
const HOUR: u64 = 3_600_000;

pub fn allowed_url(url: &Url) -> bool {
    crate::network::public_host(url) && url.username().is_empty() && url.password().is_none()
}
pub fn decode(encoded: &str) -> Result<String> {
    if encoded.len() > LIMIT {
        bail!("radio_size");
    }
    let bytes = STANDARD.decode(encoded.trim()).context("radio_encoding")?;
    let input = flate2::read::DeflateDecoder::new(bytes.as_slice());
    let mut decoded = String::new();
    input
        .take(LIMIT as u64 + 1)
        .read_to_string(&mut decoded)
        .context("radio_encoding")?;
    if decoded.len() > LIMIT {
        bail!("radio_size");
    }
    Ok(decoded)
}
#[derive(Deserialize)]
struct Document {
    countries: Vec<Country>,
}
#[derive(Deserialize)]
struct Country {
    places: Vec<Place>,
}
#[derive(Deserialize)]
struct Place {
    #[serde(default)]
    title: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    channels: Vec<Station>,
}
#[derive(Deserialize)]
struct Station {
    #[serde(default)]
    code: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default = "enabled")]
    enabled: bool,
}
fn enabled() -> bool {
    true
}
pub fn parse(text: &str) -> Result<Vec<CatalogChannel>> {
    if text.len() > LIMIT {
        bail!("radio_size");
    }
    let document: Document = serde_json::from_str(text).context("radio_json")?;
    let mut channels = Vec::new();
    let mut ids = HashSet::new();
    for country in document.countries {
        for place in country.places {
            let city = if place.title.is_empty() {
                place.name
            } else {
                place.title
            };
            let city = match city.as_str() {
                "Rome" => "Roma",
                "Florence" => "Firenze",
                "Milan" => "Milano",
                "Turin" => "Torino",
                "Venice" => "Venezia",
                "Naples" => "Napoli",
                "Genoa" => "Genova",
                "Padua" => "Padova",
                "Syracuse" => "Siracusa",
                other => other,
            }
            .chars()
            .take(160)
            .collect::<String>();
            for station in place.channels {
                if !station.enabled
                    || station.title.trim().is_empty()
                    || station.code.is_empty()
                    || station.code.len() > 100
                    || !station
                        .code
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
                {
                    continue;
                }
                let Ok(mut url) = Url::parse(&station.url) else {
                    continue;
                };
                if !allowed_url(&url) {
                    continue;
                }
                if url.host_str() == Some("radio.garden") {
                    let _ = url.set_scheme("https");
                }
                let id = format!(
                    "{:x}",
                    Sha256::digest(format!("radio|{}", station.code).as_bytes())
                )[..24]
                    .to_owned();
                if !ids.insert(id.clone()) {
                    continue;
                }
                channels.push(CatalogChannel {
                    kind: MediaKind::Radio,
                    id,
                    name: station.title.trim().chars().take(160).collect(),
                    number: 9999,
                    provider: "direct".into(),
                    epg_id: String::new(),
                    key: url.into(),
                    headers: Default::default(),
                    logo: String::new(),
                    group: city.clone(),
                    color: "#99caff".into(),
                });
            }
        }
    }
    let priority = [
        "rai radio 1",
        "rai radio 2",
        "rai radio 3",
        "radio 24",
        "rds fm 103.0",
        "radio 105 fm",
        "r101",
        "virgin radio italy",
    ];
    channels.sort_by_key(|c| {
        (
            priority
                .iter()
                .position(|v| *v == c.name.to_lowercase())
                .unwrap_or(usize::MAX),
            c.name.to_lowercase(),
            c.key.contains("radio.garden"),
        )
    });
    let mut names = HashSet::new();
    channels.retain(|c| names.insert((c.name.to_lowercase(), c.group.to_lowercase())));
    if channels.is_empty() {
        bail!("radio_empty");
    }
    Ok(channels)
}
#[derive(Clone, Default, Serialize)]
pub struct RadioStatus {
    pub updated_at: u64,
    pub next_retry_at: u64,
    pub loading: bool,
    pub error: Option<String>,
    pub revision: String,
}
#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct Cache {
    pub text: String,
    pub updated_at: u64,
    pub attempted: u64,
    pub next_retry_at: u64,
    pub error: Option<String>,
}
struct Store {
    cache: Cache,
    channels: Vec<CatalogChannel>,
}
pub struct RadioService {
    path: PathBuf,
    store: Mutex<Store>,
    loading: AtomicBool,
    client: Client,
}
impl RadioService {
    pub fn new(directory: &Path) -> Result<Arc<Self>> {
        let path = directory.join("radio-catalog.json");
        let cached = fs::File::open(&path).ok().and_then(|f| {
            let mut bytes = Vec::new();
            f.take((LIMIT * 2) as u64 + 1)
                .read_to_end(&mut bytes)
                .ok()?;
            if bytes.len() > LIMIT * 2 {
                return None;
            }
            let cache: Cache = serde_json::from_slice(&bytes).ok()?;
            let channels = parse(&cache.text).ok()?;
            Some(Store { cache, channels })
        });
        let store = cached.unwrap_or_else(|| Store {
            cache: Cache {
                text: BUNDLED.into(),
                ..Default::default()
            },
            channels: parse(BUNDLED).expect("bundled radio catalogue"),
        });
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(30))
            .referer(false)
            .redirect(reqwest::redirect::Policy::custom(|a| {
                if a.previous().len() < 3
                    && a.url().scheme() == "https"
                    && a.url().host_str() == Some("www.worldtvlive.eu")
                {
                    a.follow()
                } else {
                    a.stop()
                }
            }))
            .build()
            .map_err(|_| anyhow!("Radio non disponibili"))?;
        Ok(Arc::new(Self {
            path,
            store: Mutex::new(store),
            loading: AtomicBool::new(false),
            client,
        }))
    }
    pub fn channel(&self, id: &str) -> Option<CatalogChannel> {
        self.store
            .lock()
            .ok()?
            .channels
            .iter()
            .find(|c| c.id == id)
            .cloned()
    }
    pub fn snapshot(&self) -> (Vec<CatalogChannel>, RadioStatus) {
        let store = self.store.lock().expect("radio lock");
        (
            store.channels.clone(),
            RadioStatus {
                updated_at: store.cache.updated_at,
                next_retry_at: store.cache.next_retry_at,
                loading: self.loading.load(Ordering::Acquire),
                error: store.cache.error.clone(),
                revision: format!("{:x}", Sha256::digest(store.cache.text.as_bytes())),
            },
        )
    }
    pub fn refresh(self: &Arc<Self>, force: bool) -> bool {
        let service = self.clone();
        self.refresh_with(force, now_ms(), move || {
            let response = service.client.get(SOURCE).send()?.error_for_status()?;
            let mut text = String::new();
            response.take(LIMIT as u64 + 1).read_to_string(&mut text)?;
            decode(&text)
        })
    }
    fn refresh_with<F>(self: &Arc<Self>, force: bool, now: u64, fetch: F) -> bool
    where
        F: FnOnce() -> Result<String> + Send + 'static,
    {
        if self.loading.swap(true, Ordering::AcqRel) {
            return false;
        }
        {
            let mut store = self.store.lock().expect("radio lock");
            if store.cache.next_retry_at > now
                || (store.cache.attempted > 0 && now.saturating_sub(store.cache.attempted) < 30_000)
                || (!force
                    && store.cache.updated_at > 0
                    && now.saturating_sub(store.cache.updated_at) < DAY)
            {
                self.loading.store(false, Ordering::Release);
                return false;
            }
            store.cache.attempted = now;
            store.cache.next_retry_at = now + HOUR;
            let _ = write_cache(&self.path, &store.cache);
        }
        let service = self.clone();
        let minimum = self.store.lock().expect("radio lock").channels.len() / 2;
        std::thread::spawn(move || {
            let result = fetch().and_then(|text| {
                let channels = parse(&text)?;
                if channels.len() < minimum.max(10) {
                    bail!("radio_incomplete");
                }
                Ok((text, channels))
            });
            let mut store = service.store.lock().expect("radio lock");
            match result {
                Ok((text, channels)) => {
                    let cache = Cache {
                        text,
                        updated_at: now,
                        attempted: now,
                        next_retry_at: 0,
                        error: None,
                    };
                    if write_cache(&service.path, &cache).is_ok() {
                        store.cache = cache;
                        store.channels = channels;
                    } else {
                        store.cache.error = Some(
                            "Non riesco a salvare le radio. Mantengo il catalogo disponibile."
                                .into(),
                        );
                    }
                }
                Err(_) => {
                    store.cache.error = Some(
                        "Il catalogo radio non è raggiungibile. Puoi usare quello già disponibile."
                            .into(),
                    );
                    let _ = write_cache(&service.path, &store.cache);
                }
            }
            service.loading.store(false, Ordering::Release);
        });
        true
    }
}
fn write_cache(path: &Path, cache: &Cache) -> Result<()> {
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(&serde_json::to_vec(cache)?)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    let _ = fs::remove_file(temporary);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_catalogue_has_stable_radio_identities_and_cities() {
        let channels = parse(BUNDLED).unwrap();
        assert_eq!(channels.len(), 1310);
        assert_eq!(channels[0].name, "Rai Radio 1");
        assert!(channels
            .iter()
            .all(|c| c.kind == MediaKind::Radio && c.epg_id.is_empty()));
        assert!(channels.iter().any(|c| c.group == "Milano"));
    }
    #[test]
    fn source_address_changes_preserve_the_station_id_and_invalid_urls_are_excluded() {
        let text = |url: &str| {
            format!(
                r#"{{"countries":[{{"places":[{{"title":"Rome","channels":[{{"code":"station","title":"Radio","url":"{url}"}}]}}]}}]}}"#
            )
        };
        let a = parse(&text("https://example.com/a")).unwrap();
        let b = parse(&text("https://example.com/b")).unwrap();
        assert_eq!(a[0].id, b[0].id);
        assert!(parse(&text("http://127.0.0.1/live")).is_err());
        assert!(parse(&text("https://user:secret@example.com/live")).is_err());
    }
    #[test]
    fn feed_decode_is_bounded() {
        assert!(decode(&"a".repeat(LIMIT + 1)).is_err());
        assert!(decode("not base64").is_err());
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&vec![b'a'; LIMIT + 1]).unwrap();
        assert!(decode(&STANDARD.encode(encoder.finish().unwrap())).is_err());
    }
    #[test]
    fn incomplete_entries_do_not_discard_valid_stations() {
        let text = r#"{"countries":[{"places":[{"channels":[{}, {"code":"ok","title":"Radio","url":"https://example.com/live"}]}]}]}"#;
        assert_eq!(parse(text).unwrap().len(), 1);
    }
    #[test]
    fn failed_refresh_preserves_catalogue_and_retry_delay_after_restart() {
        let directory = std::env::temp_dir().join(format!("lume-radio-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let service = RadioService::new(&directory).unwrap();
        let now = now_ms();
        assert!(service.refresh_with(true, now, || Err(anyhow!("offline"))));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while service.loading.load(Ordering::Acquire) {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        let restored = RadioService::new(&directory).unwrap();
        let (channels, status) = restored.snapshot();
        assert_eq!(channels.len(), 1310);
        assert!(status.error.is_some());
        assert_eq!(status.next_retry_at, now + HOUR);
        assert!(!restored.refresh_with(true, now + 30_001, || panic!("cooldown ignored")));
        fs::remove_dir_all(directory).unwrap();
    }
}
