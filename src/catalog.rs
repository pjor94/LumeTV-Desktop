// SPDX-License-Identifier: GPL-3.0-or-later
//! Android-compatible M3U catalogue, with stable legacy IDs and atomic refreshes.
use crate::{model::Channel, report::now_ms};
use anyhow::{anyhow, bail, Context, Result};
use regex::Regex;
use reqwest::{blocking::Client, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::Duration,
};

pub const BUNDLED_PLAYLIST: &str = include_str!("../resources/channels.m3u");
pub const SOURCE_URL: &str = "https://tvchannels.worldtvlive.eu/tv/_dtt_kodi/list.m3u8";
const DAY: u64 = 86_400_000;
const HOUR: u64 = 3_600_000;
const MAX_PLAYLIST: usize = 2 * 1024 * 1024;
pub const PROVIDERS: &[&str] = &[
    "rai",
    "mediaset",
    "la7",
    "sky",
    "discovery",
    "direct",
    "rainewsrg",
];

#[derive(Clone, Deserialize, Serialize)]
pub struct CatalogChannel {
    #[serde(default)]
    pub kind: crate::model::MediaKind,
    pub id: String,
    pub name: String,
    pub number: u32,
    pub provider: String,
    #[serde(default)]
    pub epg_id: String,
    #[serde(skip_serializing)]
    pub key: String,
    #[serde(default, skip_serializing)]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing)]
    pub logo: String,
    pub group: String,
    pub color: String,
}
impl CatalogChannel {
    pub fn stream_channel(&self) -> Channel {
        Channel {
            kind: self.kind,
            id: self.id.clone(),
            name: self.name.clone(),
            provider: self.provider.clone(),
            key: self.key.clone(),
            headers: self.headers.clone(),
        }
    }
}
pub fn canonical_epg(value: &str) -> String {
    let value = value.trim().to_lowercase();
    match value.strip_suffix(".it").unwrap_or(&value) {
        "rairadio2visualradio" => "rairadio2".into(),
        value => value.into(),
    }
}
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 96
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
fn legacy() -> &'static [CatalogChannel] {
    static LEGACY: OnceLock<Vec<CatalogChannel>> = OnceLock::new();
    LEGACY.get_or_init(|| {
        serde_json::from_str(include_str!("../resources/catalog-legacy.json"))
            .expect("legacy catalogue")
    })
}
pub fn catalog() -> &'static [CatalogChannel] {
    static CHANNELS: OnceLock<Vec<CatalogChannel>> = OnceLock::new();
    CHANNELS.get_or_init(|| {
        parse_playlist(BUNDLED_PLAYLIST)
            .expect("bundled Android catalogue")
            .channels
    })
}
pub fn channel(id: &str) -> Option<&'static CatalogChannel> {
    catalog().iter().find(|c| c.id == id)
}
pub struct ParsedPlaylist {
    pub channels: Vec<CatalogChannel>,
    text: String,
}

fn decode(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    let mut result = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let pair = bytes.get(i + 1..i + 3).context("catalog_encoding")?;
            let a = (pair[0] as char).to_digit(16).context("catalog_encoding")?;
            let b = (pair[1] as char).to_digit(16).context("catalog_encoding")?;
            result.push((a * 16 + b) as u8);
            i += 3;
        } else {
            result.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(result).map_err(|_| anyhow!("catalog_encoding"))
}
fn add_header(headers: &mut BTreeMap<String, String>, key: &str, value: &str) -> Result<()> {
    if value.len() > 8192 || value.chars().any(char::is_control) {
        bail!("catalog_header");
    }
    let name = match key.to_ascii_lowercase().as_str() {
        "user-agent" => "User-Agent",
        "referer" | "referrer" => "Referer",
        "origin" => "Origin",
        "accept" => "Accept",
        "authorization" => "Authorization",
        "cookie" => "Cookie",
        "content-type" => "Content-Type",
        _ => return Ok(()),
    };
    headers.insert(name.into(), value.into());
    Ok(())
}
fn public_source(value: &str) -> Result<Url> {
    let url = Url::parse(value).map_err(|_| anyhow!("catalog_url"))?;
    let host = url.host_str().unwrap_or_default();
    if !["http", "https"].contains(&url.scheme())
        || !url.username().is_empty()
        || url.password().is_some()
        || !host.contains('.')
        || host.parse::<std::net::IpAddr>().is_ok()
        || [".local", ".localhost", ".internal", ".lan"]
            .iter()
            .any(|s| host.ends_with(s))
    {
        bail!("catalog_url");
    }
    Ok(url)
}
fn excluded(provider: &str, name: &str, epg_id: &str, source: &str) -> bool {
    static NAME: OnceLock<Regex> = OnceLock::new();
    static ID: OnceLock<Regex> = OnceLock::new();
    static SOURCE: OnceLock<Regex> = OnceLock::new();
    provider == "rsi"
        || NAME
            .get_or_init(|| Regex::new(r"(?i)^rsi\s*la\s*[12](?:[^a-z0-9]|$)").unwrap())
            .is_match(name.trim())
        || ID
            .get_or_init(|| Regex::new(r"(?i)^rsila[12](?:vpn|instabile|hd|it|ch)?$").unwrap())
            .is_match(
                &epg_id
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric())
                    .collect::<String>(),
            )
        || SOURCE
            .get_or_init(|| Regex::new(r"(?i)(?:^|[/=?&])rsila[12](?:hd)?(?:$|[/.?&#])").unwrap())
            .is_match(source)
}
fn entry(lines: &[String]) -> Result<Option<CatalogChannel>> {
    static ATTRIBUTE: OnceLock<Regex> = OnceLock::new();
    let info = &lines[0];
    let attrs: BTreeMap<_, _> = ATTRIBUTE
        .get_or_init(|| Regex::new(r#"([\w-]+)="([^"]*)""#).unwrap())
        .captures_iter(info)
        .map(|c| (c[1].to_owned(), c[2].to_owned()))
        .collect();
    let mut quoted = false;
    let separator = info.char_indices().find_map(|(i, c)| {
        if c == '"' {
            quoted = !quoted;
        }
        (c == ',' && !quoted).then_some(i)
    });
    let name = separator
        .map(|i| info[i + 1..].trim())
        .unwrap_or_else(|| attrs.get("tvg-name").map(String::as_str).unwrap_or(""));
    if name.is_empty() || name.chars().count() > 240 {
        bail!("catalog_name");
    }
    let source = lines
        .iter()
        .skip(1)
        .find(|s| !s.starts_with('#') && !s.is_empty())
        .context("catalog_truncated")?;
    let (url, inline) = source.split_once('|').unwrap_or((source, ""));
    if url.len() > 8192 {
        bail!("catalog_url");
    }
    let (provider, key) = if url.starts_with("http://") || url.starts_with("https://") {
        ("direct".to_owned(), public_source(url)?.to_string())
    } else if let Some(route) = url.strip_prefix("plugin://plugin.video.wltvhelper/") {
        let Some(route) = route.trim_start_matches(['?', '/']).strip_prefix("play/") else {
            return Ok(None);
        };
        let Some((provider, key)) = route.split_once('/') else {
            return Ok(None);
        };
        (provider.to_owned(), decode(key)?)
    } else {
        return Ok(None);
    };
    let epg_id = attrs.get("tvg-id").cloned().unwrap_or_default();
    if !PROVIDERS.contains(&provider.as_str()) || excluded(&provider, name, &epg_id, url) {
        return Ok(None);
    }
    if epg_id.len() > 128 || key.is_empty() {
        bail!("catalog_identity");
    }
    let raw_group = attrs
        .get("group-title")
        .map(String::as_str)
        .unwrap_or("Altri");
    let group = if provider == "rainewsrg" {
        "Regionali"
    } else {
        match raw_group {
            "Movie/Fiction" => "Cinema e serie",
            "Doc/Cultura" => "Cultura",
            "News" => "Notizie",
            "Grande Fratello" => "Intrattenimento",
            "Unknown" => "Altri",
            value => value,
        }
    };
    if group.len() > 100 {
        bail!("catalog_group");
    }
    let identity = if provider == "direct" {
        format!("{epg_id}|{name}|{raw_group}")
    } else {
        format!("{provider}|{key}")
    };
    let old = legacy()
        .iter()
        .find(|c| c.provider == provider && c.key == key);
    let id = old
        .map(|c| c.id.clone())
        .unwrap_or_else(|| format!("{:x}", Sha256::digest(identity.as_bytes()))[..24].to_owned());
    let color = old
        .map(|c| c.color.as_str())
        .unwrap_or(match provider.as_str() {
            "rai" => "#554ee5",
            "rainewsrg" => "#329765",
            "mediaset" => "#346da8",
            "la7" => "#e5b947",
            "sky" => "#308bc9",
            "discovery" => "#657cd4",
            _ => "#7e91ac",
        });
    let mut headers = BTreeMap::new();
    for (attr, header) in [
        ("http-user-agent", "User-Agent"),
        ("http-referrer", "Referer"),
    ] {
        if let Some(v) = attrs.get(attr) {
            add_header(&mut headers, header, v)?;
        }
    }
    for line in lines {
        for (prefix, header) in [
            ("#EXTVLCOPT:http-user-agent=", "User-Agent"),
            ("#EXTVLCOPT:http-referrer=", "Referer"),
        ] {
            if let Some(v) = line.strip_prefix(prefix) {
                add_header(&mut headers, header, v)?;
            }
        }
    }
    for v in inline.split('&').filter(|s| !s.is_empty()) {
        if let Some((k, v)) = v.split_once('=') {
            add_header(&mut headers, &decode(k)?, &decode(v)?)?;
        }
    }
    Ok(Some(CatalogChannel {
        kind: crate::model::MediaKind::Tv,
        id,
        name: name.into(),
        number: attrs
            .get("tvg-chno")
            .and_then(|s| s.parse().ok())
            .unwrap_or(9999),
        provider,
        epg_id,
        key,
        headers,
        logo: attrs
            .get("tvg-logo")
            .cloned()
            .filter(|s| s.len() <= 2048)
            .unwrap_or_default(),
        group: group.into(),
        color: color.into(),
    }))
}

pub fn parse_playlist(text: &str) -> Result<ParsedPlaylist> {
    if text.len() > MAX_PLAYLIST
        || !text
            .trim_start_matches('\u{feff}')
            .trim_start()
            .starts_with("#EXTM3U")
    {
        bail!("catalog_format");
    }
    let mut channels = Vec::new();
    let mut kept = String::from("#EXTM3U\n");
    let mut block = Vec::new();
    let mut pending = false;
    let mut seen = HashSet::new();
    let mut entries = 0;
    let flush = |block: &mut Vec<String>,
                 channels: &mut Vec<CatalogChannel>,
                 kept: &mut String,
                 seen: &mut HashSet<String>|
     -> Result<()> {
        if !block.is_empty() {
            if let Some(channel) = entry(block)? {
                if seen.insert(channel.id.clone()) {
                    channels.push(channel);
                    kept.push_str(&block.join("\n"));
                    kept.push('\n');
                }
            }
            block.clear();
        }
        Ok(())
    };
    for line in text.trim_start_matches('\u{feff}').lines().map(str::trim) {
        if line.starts_with("#EXTINF:") {
            if pending {
                bail!("catalog_truncated");
            }
            flush(&mut block, &mut channels, &mut kept, &mut seen)?;
            entries += 1;
            if entries > 4096 {
                bail!("catalog_entries");
            }
            pending = true;
            block.push(line.into());
        } else if !line.is_empty() && !line.starts_with('#') {
            if !pending {
                bail!("catalog_structure");
            }
            pending = false;
            block.push(line.into());
        } else if !block.is_empty() {
            block.push(line.into());
        }
    }
    if pending {
        bail!("catalog_truncated");
    }
    flush(&mut block, &mut channels, &mut kept, &mut seen)?;
    if channels.len() < 20
        || channels.iter().filter(|c| c.provider == "rai").count() < 3
        || channels.iter().filter(|c| c.provider == "mediaset").count() < 3
    {
        bail!("catalog_incomplete");
    }
    channels.sort_by_key(|c| c.number);
    Ok(ParsedPlaylist {
        channels,
        text: kept,
    })
}

#[derive(Clone, Default, Serialize)]
pub struct CatalogStatus {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radio: Option<crate::radio::RadioStatus>,
    pub updated_at: u64,
    pub last_attempt: u64,
    pub next_retry_at: u64,
    pub loading: bool,
    pub error: Option<String>,
    pub revision: String,
}
#[derive(Clone, Serialize)]
pub struct CatalogSnapshot {
    pub channels: Vec<CatalogChannel>,
    #[serde(flatten)]
    pub status: CatalogStatus,
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct Cache {
    playlist: String,
    last_success: u64,
    last_attempt: u64,
    next_retry_at: u64,
    error: Option<String>,
}
struct Store {
    cache: Cache,
    parsed: ParsedPlaylist,
}
pub struct CatalogService {
    pub radios: Arc<crate::radio::RadioService>,
    store: Mutex<Store>,
    loading: AtomicBool,
    path: PathBuf,
    client: Client,
}

impl CatalogService {
    pub fn new(directory: &Path) -> Result<Arc<Self>> {
        fs::create_dir_all(directory).context("Impossibile preparare il catalogo")?;
        let path = directory.join("catalog.json");
        let cached = fs::File::open(&path).ok().and_then(|f| {
            let mut bytes = Vec::new();
            f.take(8 * 1024 * 1024 + 1).read_to_end(&mut bytes).ok()?;
            if bytes.len() > 8 * 1024 * 1024 {
                return None;
            }
            let cache: Cache = serde_json::from_slice(&bytes).ok()?;
            let parsed = parse_playlist(&cache.playlist).ok()?;
            Some(Store { cache, parsed })
        });
        let store = cached.unwrap_or_else(|| {
            let parsed = parse_playlist(BUNDLED_PLAYLIST).expect("bundled catalogue");
            Store {
                cache: Cache {
                    playlist: parsed.text.clone(),
                    ..Default::default()
                },
                parsed,
            }
        });
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .referer(false)
            .user_agent(crate::providers::USER_AGENT)
            .redirect(reqwest::redirect::Policy::custom(|a| {
                if a.previous().len() < 3
                    && a.url().scheme() == "https"
                    && a.url().host_str() == Some("tvchannels.worldtvlive.eu")
                {
                    a.follow()
                } else {
                    a.stop()
                }
            }))
            .build()
            .map_err(|_| anyhow!("Catalogo non disponibile"))?;
        Ok(Arc::new(Self {
            radios: crate::radio::RadioService::new(directory)?,
            store: Mutex::new(store),
            loading: AtomicBool::new(false),
            path,
            client,
        }))
    }
    pub fn snapshot(&self) -> CatalogSnapshot {
        let store = self.store.lock().expect("catalog lock");
        CatalogSnapshot {
            channels: store.parsed.channels.clone(),
            status: CatalogStatus {
                radio: None,
                updated_at: store.cache.last_success,
                last_attempt: store.cache.last_attempt,
                next_retry_at: store.cache.next_retry_at,
                loading: self.loading.load(Ordering::Acquire),
                error: store.cache.error.clone(),
                revision: format!("{:x}", Sha256::digest(store.parsed.text.as_bytes())),
            },
        }
    }
    pub fn channel(&self, id: &str) -> Option<CatalogChannel> {
        self.store
            .lock()
            .ok()?
            .parsed
            .channels
            .iter()
            .find(|c| c.id == id)
            .cloned()
            .or_else(|| self.radios.channel(id))
    }
    pub fn desktop_snapshot(&self) -> CatalogSnapshot {
        let mut snapshot = self.snapshot();
        let (channels, status) = self.radios.snapshot();
        snapshot.channels.extend(channels);
        snapshot.status.loading |= status.loading;
        snapshot.status.revision.push_str(&status.revision);
        snapshot.status.radio = Some(status);
        snapshot
    }
    pub fn guide_signature(&self) -> String {
        guide_signature(&self.store.lock().expect("catalog lock").parsed.channels)
    }
    pub fn refresh(self: &Arc<Self>, force: bool) -> bool {
        let service = self.clone();
        self.refresh_with(force, now_ms(), move || service.fetch())
    }
    fn refresh_with<F>(self: &Arc<Self>, force: bool, now: u64, fetch: F) -> bool
    where
        F: FnOnce() -> Result<String> + Send + 'static,
    {
        if self
            .loading
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            return false;
        }
        let recent =
            |saved: u64, duration: u64| saved > 0 && now >= saved && now - saved < duration;
        {
            let mut store = self.store.lock().expect("catalog lock");
            if (!force && recent(store.cache.last_success, DAY))
                || recent(store.cache.last_attempt, 30_000)
                || (store.cache.last_attempt <= now && store.cache.next_retry_at > now)
            {
                self.loading.store(false, Ordering::Release);
                return false;
            }
            store.cache.last_attempt = now;
            if write_cache(&self.path, &store.cache).is_err() {
                store.cache.error =
                    Some("Non riesco a salvare il catalogo. I canali restano disponibili.".into());
                store.cache.next_retry_at = now.saturating_add(HOUR);
                self.loading.store(false, Ordering::Release);
                return false;
            }
        }
        let service = self.clone();
        std::thread::spawn(move || {
            let result = fetch().and_then(|body| parse_playlist(&body));
            let mut store = service.store.lock().expect("catalog lock");
            let result = result.and_then(|parsed| {
                if parsed.channels.len() < store.parsed.channels.len() / 2 {
                    bail!("catalog_incomplete");
                }
                let cache = Cache {
                    playlist: parsed.text.clone(),
                    last_success: now,
                    last_attempt: now,
                    ..Default::default()
                };
                write_cache(&service.path, &cache)?;
                store.cache = cache;
                store.parsed = parsed;
                Ok(())
            });
            if let Err(error) = result {
                let retry = error
                    .downcast_ref::<RateLimit>()
                    .map_or(HOUR, |limit| limit.0);
                store.cache.next_retry_at = now.saturating_add(retry);
                store.cache.error = Some("Non riesco ad aggiornare la lista. Mantengo i canali salvati e riproverò più tardi.".into());
                let _ = write_cache(&service.path, &store.cache);
            }
            service.loading.store(false, Ordering::Release);
        });
        true
    }
    fn fetch(&self) -> Result<String> {
        let response = self
            .client
            .get(SOURCE_URL)
            .send()
            .map_err(|_| anyhow!("catalog_network"))?;
        if response.status().as_u16() == 429 {
            let seconds = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(3600)
                .clamp(60, 86_400);
            return Err(RateLimit(seconds * 1000).into());
        }
        if !response.status().is_success() {
            bail!("catalog_response");
        }
        let mut bytes = Vec::new();
        response
            .take(MAX_PLAYLIST as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| anyhow!("catalog_download"))?;
        if bytes.len() > MAX_PLAYLIST {
            bail!("catalog_size");
        }
        String::from_utf8(bytes).map_err(|_| anyhow!("catalog_encoding"))
    }
}
pub fn guide_signature(channels: &[CatalogChannel]) -> String {
    let mut ids: Vec<_> = channels
        .iter()
        .map(|c| format!("{}:{}", c.id, canonical_epg(&c.epg_id)))
        .collect();
    ids.sort();
    format!("{:x}", Sha256::digest(ids.join("\n").as_bytes()))
}
#[derive(Debug)]
struct RateLimit(u64);
impl std::fmt::Display for RateLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("catalog_rate_limit")
    }
}
impl std::error::Error for RateLimit {}
fn write_cache(path: &Path, cache: &Cache) -> Result<()> {
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut options = fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| -> Result<()> {
        let mut file = options.open(&temporary)?;
        file.write_all(&serde_json::to_vec(cache)?)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result.map_err(|_| anyhow!("catalog_cache_write"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn directory() -> PathBuf {
        let path = std::env::temp_dir().join(format!("lume-catalog-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        path
    }
    fn finish(service: &CatalogService) {
        let limit = std::time::Instant::now() + Duration::from_secs(3);
        while service.loading.load(Ordering::Acquire) {
            assert!(
                std::time::Instant::now() < limit,
                "catalog worker timed out"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn bundled_android_catalogue_has_all_126_channels_and_preserves_every_legacy_id() {
        let channels = catalog();
        assert_eq!(channels.len(), 126);
        let mut counts = BTreeMap::<&str, usize>::new();
        for channel in channels {
            *counts.entry(&channel.provider).or_default() += 1;
            assert!(valid_id(&channel.id));
        }
        assert_eq!(
            counts,
            BTreeMap::from([
                ("rai", 21),
                ("mediaset", 23),
                ("la7", 2),
                ("sky", 3),
                ("discovery", 8),
                ("direct", 47),
                ("rainewsrg", 22)
            ])
        );
        assert_eq!(
            channels.iter().map(|c| &c.id).collect::<HashSet<_>>().len(),
            126
        );
        for old in legacy() {
            let retained = channel(&old.id).unwrap();
            assert!(retained.provider == old.provider && retained.key == old.key);
        }
        assert_eq!(channel("la7cinema").unwrap().key, "La7c");
        let json = serde_json::to_value(channels).unwrap();
        for item in json.as_array().unwrap() {
            for field in ["key", "headers", "logo"] {
                assert!(item.get(field).is_none());
            }
        }
        let direct = channels.iter().find(|c| c.provider == "direct").unwrap();
        let diagnostic = serde_json::to_string(&direct.stream_channel()).unwrap();
        assert!(!diagnostic.contains(&direct.key));
    }
    #[test]
    fn names_quoted_attributes_headers_and_plus_signs_match_android_without_exposing_sources() {
        let input=format!("{BUNDLED_PLAYLIST}\n#EXTINF:-1 tvg-id=\"fixture\" group-title=\"Music, other\",A name, with commas\n#EXTVLCOPT:http-user-agent=original\nhttps://media.example/live.m3u8|User-Agent=a+b%2Fc&Referer=https%3A%2F%2Fwww.example%2F&Host=forbidden\n");
        let parsed = parse_playlist(&input).unwrap();
        let item = parsed
            .channels
            .iter()
            .find(|c| c.epg_id == "fixture")
            .unwrap();
        assert_eq!(item.name, "A name, with commas");
        assert_eq!(item.headers["User-Agent"], "a+b/c");
        assert_eq!(item.headers["Referer"], "https://www.example/");
        assert!(!item.headers.contains_key("Host"));
        assert!(parse_playlist(&input.replace("a+b%2Fc", "bad%0D%0Aheader")).is_err());
    }
    #[test]
    fn incomplete_even_excluded_entries_never_replace_a_catalogue() {
        for input in [
            format!("{BUNDLED_PLAYLIST}\n#EXTINF:-1,broken\n"),
            format!("{BUNDLED_PLAYLIST}\n#EXTINF:-1 tvg-id=\"rsila1\",RSI La1\n"),
            "<html>error</html>".into(),
            "#EXTM3U\nhttps://media.example/live.m3u8".into(),
        ] {
            assert!(parse_playlist(&input).is_err());
        }
        let excluded = format!("{BUNDLED_PLAYLIST}\n#EXTINF:-1 tvg-id=\"rsi-la-1-hd\",Alias\nhttps://media.example/rsila1/live.m3u8\n");
        let parsed = parse_playlist(&excluded).unwrap();
        assert_eq!(parsed.channels.len(), 126);
        assert!(!parsed.text.contains("Alias"));
    }
    #[test]
    fn updated_names_keep_plugin_ids_and_duplicate_routes_are_deduplicated() {
        let changed = BUNDLED_PLAYLIST.replace(",Rai 1\n", ",Rai Uno\n");
        let parsed=parse_playlist(&format!("{changed}\n#EXTINF:-1,Rai duplicate\nplugin://plugin.video.wltvhelper/play/rai/Rai%201\n")).unwrap();
        assert_eq!(parsed.channels.len(), 126);
        assert_eq!(
            parsed
                .channels
                .iter()
                .find(|c| c.id == "rai1")
                .unwrap()
                .name,
            "Rai Uno"
        );
    }
    #[test]
    fn daily_refresh_coalesces_requests_persists_success_and_supports_manual_refresh() {
        let dir = directory();
        let service = CatalogService::new(&dir).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let now = 1_000_000;
        for _ in 0..4 {
            let calls = calls.clone();
            service.refresh_with(false, now, move || {
                calls.fetch_add(1, Ordering::Relaxed);
                Ok(BUNDLED_PLAYLIST.replace(",Rai 1\n", ",Rai Uno\n"))
            });
        }
        finish(&service);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(service.snapshot().status.updated_at, now);
        let restored = CatalogService::new(&dir).unwrap();
        assert_eq!(restored.channel("rai1").unwrap().name, "Rai Uno");
        assert!(!restored.refresh_with(false, now + DAY - 1, || panic!("not due")));
        assert!(restored.refresh_with(true, now + 60_000, || Ok(BUNDLED_PLAYLIST.into())));
        finish(&restored);
        assert_eq!(restored.channel("rai1").unwrap().name, "Rai 1");
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn offline_or_invalid_updates_preserve_the_cache_and_the_retry_survives_restart() {
        let dir = directory();
        let service = CatalogService::new(&dir).unwrap();
        let now = 1_000_000;
        assert!(service.refresh_with(false, now, || Ok(BUNDLED_PLAYLIST.into())));
        finish(&service);
        let revision = service.snapshot().status.revision;
        assert!(service.refresh_with(true, now + 60_000, || Ok("<html>partial</html>".into())));
        finish(&service);
        let restored = CatalogService::new(&dir).unwrap();
        let status = restored.snapshot().status;
        assert_eq!(status.revision, revision);
        assert_eq!(status.updated_at, now);
        assert!(status.error.is_some());
        assert!(!restored.refresh_with(true, now + 120_000, || panic!("must respect retry")));
        assert!(restored.refresh_with(true, now + 60_000 + HOUR, || Err(anyhow!("offline"))));
        finish(&restored);
        assert_eq!(restored.snapshot().channels.len(), 126);
        // A clock corrected backwards must not freeze the list indefinitely.
        assert!(restored.refresh_with(false, 10, || Ok(BUNDLED_PLAYLIST.into())));
        finish(&restored);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn corrupt_cache_falls_back_to_bundle_and_failed_writes_never_record_a_success() {
        let dir = directory();
        fs::write(dir.join("catalog.json"), b"corrupt").unwrap();
        let service = CatalogService::new(&dir).unwrap();
        assert_eq!(service.snapshot().channels.len(), 126);
        fs::remove_file(dir.join("catalog.json")).unwrap();
        fs::create_dir(dir.join("catalog.json")).unwrap();
        assert!(!service.refresh_with(false, 1_000_000, || panic!("cannot persist attempt")));
        assert_eq!(service.snapshot().status.updated_at, 0);
        assert!(service.snapshot().status.error.is_some());
        fs::remove_dir_all(dir).unwrap();
    }
}
