// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded XMLTV/XZ ingestion of the same public EPG used by Lume TV Android.
use crate::{
    catalog::{canonical_epg, catalog, guide_signature, CatalogChannel, CatalogService},
    report::now_ms,
};
use anyhow::{anyhow, bail, Context, Result};
use lzma_rust2::XzReader;
use quick_xml::{
    events::{BytesStart, Event},
    Reader,
};
use reqwest::{blocking::Client, Url};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{BufRead, BufReader, Read},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, RwLock,
    },
    time::Duration,
};

const FEED: &str = "https://api.epg-guide.com/get/wltv.xz";
const HOUR: u64 = 3_600_000;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Programme {
    pub title: String,
    pub start: u64,
    pub end: u64,
    pub description: String,
    pub category: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct GuideSnapshot {
    pub catalog_signature: String,
    pub updated_at: u64,
    pub programmes: BTreeMap<String, Vec<Programme>>,
    pub logos: BTreeMap<String, String>,
    pub loading: bool,
    pub error: Option<String>,
    pub next_retry_at: u64,
}

pub struct GuideService {
    catalog: Arc<CatalogService>,
    snapshot: RwLock<GuideSnapshot>,
    loading: AtomicBool,
    attempted: AtomicU64,
    cache: PathBuf,
    client: Client,
}

impl GuideService {
    pub fn new(directory: &std::path::Path) -> Result<Arc<Self>> {
        Self::with_catalog(directory, CatalogService::new(directory)?)
    }
    pub fn with_catalog(
        directory: &std::path::Path,
        catalog: Arc<CatalogService>,
    ) -> Result<Arc<Self>> {
        let cache = directory.join("guide.json");
        let mut snapshot: GuideSnapshot = fs::read(&cache)
            .ok()
            .filter(|b| b.len() <= 12 * 1024 * 1024)
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        snapshot.loading = false;
        let client = Client::builder()
            .timeout(Duration::from_secs(90))
            .connect_timeout(Duration::from_secs(10))
            .referer(false)
            .user_agent("LumeTV/0.2 desktop")
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                let url = attempt.url();
                if attempt.previous().len() < 3
                    && url.scheme() == "https"
                    && url
                        .host_str()
                        .is_some_and(|h| h == "epg-guide.com" || h.ends_with(".epg-guide.com"))
                {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }))
            .build()
            .map_err(|_| anyhow!("Impossibile preparare la guida TV"))?;
        Ok(Arc::new(Self {
            catalog,
            snapshot: RwLock::new(snapshot),
            loading: AtomicBool::new(false),
            attempted: AtomicU64::new(0),
            cache,
            client,
        }))
    }

    pub fn snapshot(&self) -> GuideSnapshot {
        let mut snapshot = self.snapshot.read().expect("guide lock").clone();
        snapshot.loading = self.loading.load(Ordering::Relaxed);
        snapshot
    }

    pub fn refresh(self: &Arc<Self>, force: bool) {
        let now = now_ms();
        if now < self.snapshot.read().expect("guide lock").next_retry_at {
            return;
        }
        let signature = self.catalog.guide_signature();
        let fresh = {
            let snapshot = self.snapshot.read().expect("guide lock");
            now.saturating_sub(snapshot.updated_at) < 4 * HOUR
                && snapshot.catalog_signature == signature
        };
        if (!force && fresh)
            || now.saturating_sub(self.attempted.load(Ordering::Relaxed)) < 30_000
            || self.loading.swap(true, Ordering::SeqCst)
        {
            return;
        }
        self.attempted.store(now, Ordering::Relaxed);
        let service = self.clone();
        std::thread::spawn(move || {
            match service.fetch() {
                Ok(snapshot) => {
                    if let Ok(bytes) = serde_json::to_vec(&snapshot) {
                        let temporary = service.cache.with_extension("json.tmp");
                        if fs::write(&temporary, bytes).is_ok() {
                            let _ = fs::rename(temporary, &service.cache);
                        }
                    }
                    *service.snapshot.write().expect("guide lock") = snapshot;
                }
                Err(error) => {
                    let mut snapshot = service.snapshot.write().expect("guide lock");
                    let limited = error.downcast_ref::<GuideRateLimit>();
                    snapshot.error = Some(if limited.is_some() {
                        "Il servizio della guida richiede una pausa. Lume riproverà automaticamente; i canali restano disponibili."
                    } else { "La guida non è raggiungibile. I canali restano disponibili; riprova tra poco." }.into());
                    snapshot.next_retry_at =
                        now_ms() + limited.map_or(5 * 60_000, |limit| limit.seconds * 1000);
                    if let Ok(bytes) = serde_json::to_vec(&*snapshot) {
                        let _ = fs::write(&service.cache, bytes);
                    }
                }
            }
            service.loading.store(false, Ordering::SeqCst);
        });
    }

    fn fetch(&self) -> Result<GuideSnapshot> {
        let response = self
            .client
            .get(FEED)
            .send()
            .map_err(|_| anyhow!("guide_network"))?;
        if response.status().as_u16() == 429 {
            let seconds = response
                .headers()
                .get("Retry-After")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(3600)
                .clamp(60, 86_400);
            return Err(GuideRateLimit { seconds }.into());
        }
        if !response.status().is_success()
            || response
                .content_length()
                .is_some_and(|n| n > 32 * 1024 * 1024)
        {
            bail!("guide_response");
        }
        // Complete the bounded HTTP transfer before the decoder performs small
        // reads. Byte-sized reads from reqwest's blocking bridge are costly.
        let mut compressed = Vec::new();
        response
            .take(32 * 1024 * 1024 + 1)
            .read_to_end(&mut compressed)
            .map_err(|_| anyhow!("guide_download"))?;
        if compressed.len() > 32 * 1024 * 1024 {
            bail!("guide_size_limit");
        }
        let decoder = XzReader::new_mem_limit(compressed.as_slice(), false, 64 * 1024);
        let xml = BufReader::new(LimitedRead::new(decoder, 350 * 1024 * 1024));
        parse_for_catalog(xml, now_ms(), &self.catalog.snapshot().channels)
    }

    pub fn logo_url(&self, id: &str) -> Option<String> {
        let snapshot = self.snapshot.read().ok()?;
        let channel = self.catalog.channel(id)?;
        snapshot
            .logos
            .get(id)
            .or_else(|| {
                if channel.provider == "rainewsrg" {
                    snapshot.logos.get("rai3")
                } else {
                    None
                }
            })
            .cloned()
            .or_else(|| allowed_logo(&channel.logo).then_some(channel.logo))
            .filter(|url| allowed_logo(url))
    }
}

#[derive(Debug)]
struct GuideRateLimit {
    seconds: u64,
}
impl std::fmt::Display for GuideRateLimit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("guide_rate_limit")
    }
}
impl std::error::Error for GuideRateLimit {}

pub fn allowed_logo(value: &str) -> bool {
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    value.len() < 2048
        && ["http", "https"].contains(&url.scheme())
        && [
            Some("epg-guide.com"),
            Some("www.epg-guide.com"),
            Some("116.202.210.205"),
        ]
        .contains(&url.host_str())
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url.port().is_none()
        && url.path().starts_with("/icn/icons/")
        && url.path().ends_with(".png")
}

fn canonical(value: &str) -> String {
    canonical_epg(value)
}

fn attribute(e: &BytesStart<'_>, name: &[u8]) -> String {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name)
        .and_then(|a| a.unescape_value().ok().map(|v| v.into_owned()))
        .unwrap_or_default()
}

pub fn parse<R: BufRead>(input: R, now: u64) -> Result<GuideSnapshot> {
    parse_for_catalog(input, now, catalog())
}

pub fn parse_for_catalog<R: BufRead>(
    input: R,
    now: u64,
    channels: &[CatalogChannel],
) -> Result<GuideSnapshot> {
    let mut mapping = BTreeMap::<String, Vec<String>>::new();
    for channel in channels {
        let epg = canonical(if channel.epg_id.is_empty() {
            &channel.id
        } else {
            &channel.epg_id
        });
        mapping.entry(epg).or_default().push(channel.id.clone());
    }
    let mut result = GuideSnapshot {
        catalog_signature: guide_signature(channels),
        updated_at: now,
        ..Default::default()
    };
    let mut reader = Reader::from_reader(input);
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut id = String::new();
    let mut logo_id = String::new();
    let mut field = String::new();
    let mut italian = false;
    let mut programme = Programme::default();
    loop {
        match reader.read_event_into(&mut buffer).context("guide_xml")? {
            Event::Start(e) if e.name().as_ref() == b"channel" => {
                logo_id = canonical(&attribute(&e, b"id"));
                if !mapping.contains_key(&logo_id) {
                    logo_id.clear();
                }
            }
            Event::Empty(e) if e.name().as_ref() == b"icon" && !logo_id.is_empty() => {
                let url = attribute(&e, b"src");
                if allowed_logo(&url) {
                    for id in mapping.get(&logo_id).into_iter().flatten() {
                        result
                            .logos
                            .entry(id.clone())
                            .or_insert_with(|| url.clone());
                    }
                }
            }
            Event::Start(e) if e.name().as_ref() == b"programme" => {
                id = canonical(&attribute(&e, b"channel"));
                programme = Programme {
                    start: xmltv_timestamp(&attribute(&e, b"start")).unwrap_or(0),
                    end: xmltv_timestamp(&attribute(&e, b"stop")).unwrap_or(0),
                    ..Default::default()
                };
                if !mapping.contains_key(&id)
                    || programme.end <= now.saturating_sub(6 * HOUR)
                    || programme.start >= now + 48 * HOUR
                {
                    id.clear();
                }
            }
            Event::Start(e)
                if !id.is_empty()
                    && [b"title".as_slice(), b"desc", b"category"].contains(&e.name().as_ref()) =>
            {
                field = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                italian = attribute(&e, b"lang") == "it";
            }
            Event::Text(e) if !id.is_empty() && !field.is_empty() => {
                set_text(
                    &mut programme,
                    &field,
                    &e.unescape().context("guide_text")?,
                    italian,
                );
            }
            Event::CData(e) if !id.is_empty() && !field.is_empty() => {
                set_text(
                    &mut programme,
                    &field,
                    &String::from_utf8_lossy(&e),
                    italian,
                );
            }
            Event::End(e) if e.name().as_ref() == b"channel" => logo_id.clear(),
            Event::End(e) if e.name().as_ref() == b"programme" => {
                if !id.is_empty()
                    && programme.end > programme.start
                    && programme.start > 0
                    && !programme.title.trim().is_empty()
                {
                    for target in mapping.get(&id).into_iter().flatten() {
                        let list = result.programmes.entry(target.clone()).or_default();
                        if list.len() < 192
                            && !list
                                .iter()
                                .any(|p| p.start == programme.start && p.title == programme.title)
                        {
                            list.push(programme.clone());
                        }
                    }
                }
                id.clear();
                field.clear();
            }
            Event::End(_) => field.clear(),
            Event::DocType(_) => bail!("guide_doctype_unsupported"),
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    for list in result.programmes.values_mut() {
        list.sort_by_key(|p| p.start);
    }
    if result.programmes.is_empty() {
        bail!("guide_empty");
    }
    Ok(result)
}

fn set_text(p: &mut Programme, field: &str, text: &str, italian: bool) {
    let (target, maximum) = match field {
        "title" => (&mut p.title, 300),
        "desc" => (&mut p.description, 1600),
        "category" => (&mut p.category, 100),
        _ => return,
    };
    if target.is_empty() || italian {
        *target = text.trim().chars().take(maximum).collect();
    }
}

/// XMLTV uses an explicit UTC offset, so DST follows the feed, not the host OS.
pub fn xmltv_timestamp(value: &str) -> Option<u64> {
    let mut parts = value.split_whitespace();
    let date = parts.next()?;
    let offset = parts.next().unwrap_or("+0000");
    if date.len() != 14
        || !date.bytes().all(|v| v.is_ascii_digit())
        || offset.len() != 5
        || !matches!(offset.as_bytes()[0], b'+' | b'-')
        || !offset[1..].bytes().all(|v| v.is_ascii_digit())
    {
        return None;
    }
    let n = |a, b| date.get(a..b)?.parse::<i64>().ok();
    let (year, month, day, hour, minute, second) = (
        n(0, 4)?,
        n(4, 6)?,
        n(6, 8)?,
        n(8, 10)?,
        n(10, 12)?,
        n(12, 14)?,
    );
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1970..=2100).contains(&year)
        || !(1..=12).contains(&month)
        || day < 1
        || day > days[(month - 1) as usize]
        || !(0..24).contains(&hour)
        || !(0..60).contains(&minute)
        || !(0..60).contains(&second)
    {
        return None;
    }
    let offset_h = offset[1..3].parse::<i64>().ok()?;
    let offset_m = offset[3..5].parse::<i64>().ok()?;
    if offset_h > 23 || offset_m > 59 {
        return None;
    }
    let y = year - i64::from(month <= 2);
    let era = y / 400;
    let yoe = y - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let civil_days = era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468;
    let offset_s = (offset_h * 3600 + offset_m * 60) * if offset.starts_with('-') { -1 } else { 1 };
    let timestamp = (civil_days * 86400 + hour * 3600 + minute * 60 + second - offset_s) * 1000;
    u64::try_from(timestamp).ok()
}

struct LimitedRead<R> {
    inner: R,
    remaining: u64,
}
impl<R> LimitedRead<R> {
    fn new(inner: R, remaining: u64) -> Self {
        Self { inner, remaining }
    }
}
impl<R: Read> Read for LimitedRead<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            let mut extra = [0];
            return if self.inner.read(&mut extra)? == 0 {
                Ok(0)
            } else {
                Err(std::io::Error::other("guide_size_limit"))
            };
        }
        let length = buffer.len().min(self.remaining as usize);
        let read = self.inner.read(&mut buffer[..length])?;
        self.remaining -= read as u64;
        Ok(read)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_catalogue_ids_map_to_their_xmltv_ids_and_invalidate_older_guide_caches() {
        let channels = catalog();
        let tv8 = channels.iter().find(|c| c.epg_id == "tv8").unwrap();
        let xml=br#"<tv><channel id="TV8.it"><icon src="https://epg-guide.com/icn/icons/tv8.png"/></channel><programme channel="TV8.it" start="20261108080000 +0100" stop="20261108100000 +0100"><title>Programme</title></programme></tv>"#;
        let guide = parse_for_catalog(
            xml.as_slice(),
            xmltv_timestamp("20261108090000 +0100").unwrap(),
            channels,
        )
        .unwrap();
        assert_eq!(guide.programmes[&tv8.id][0].title, "Programme");
        assert!(guide.logos.contains_key(&tv8.id));
        assert_eq!(guide.catalog_signature, guide_signature(channels));
        let mut renamed = channels.to_vec();
        renamed[0].name = "New display name".into();
        assert_eq!(guide_signature(&renamed), guide.catalog_signature);
        renamed.pop();
        assert_ne!(guide_signature(&renamed), guide.catalog_signature);
    }
    #[test]
    fn timestamps_apply_offsets_validate_dates_and_do_not_depend_on_local_dst() {
        assert_eq!(xmltv_timestamp("19700101010000 +0100"), Some(0));
        assert_eq!(
            xmltv_timestamp("20261108010000 +0100"),
            xmltv_timestamp("20261108000000 +0000")
        );
        assert_eq!(
            xmltv_timestamp("20260329030000 +0200"),
            xmltv_timestamp("20260329010000 +0000")
        );
        for date in [
            "20260230010000 +0100",
            "20261301010000 +0100",
            "20260101240000 +0100",
            "20260101010000 +2500",
            "secret",
        ] {
            assert_eq!(xmltv_timestamp(date), None);
        }
    }
    #[test]
    fn parser_maps_android_ids_prefers_italian_and_rejects_untrusted_artwork() {
        let xml = br#"<tv><channel id="Rai1.it"><icon src="http://epg-guide.com/icn/icons/rai1.png"/></channel><channel id="Rai2.it"><icon src="http://127.0.0.1/private.png"/></channel><programme channel="Rai1.it" start="20261108080000 +0100" stop="20261108100000 +0100"><title lang="en">English</title><title lang="it">Titolo &amp; titolo</title><desc><![CDATA[Descrizione]]></desc></programme><programme channel="Unknown.it" start="20261108080000 +0100" stop="20261108100000 +0100"><title>Ignored</title></programme></tv>"#;
        let guide = parse(
            xml.as_slice(),
            xmltv_timestamp("20261108090000 +0100").unwrap(),
        )
        .unwrap();
        assert_eq!(guide.programmes.len(), 1);
        assert_eq!(guide.programmes["rai1"][0].title, "Titolo & titolo");
        assert_eq!(guide.programmes["rai1"][0].description, "Descrizione");
        assert_eq!(guide.logos.len(), 1);
        assert!(crate::catalog::catalog().iter().all(|c| c.id != "Unknown"));
        assert!(!allowed_logo(
            "https://epg-guide.com@localhost/icn/icons/test.png"
        ));
    }
}
