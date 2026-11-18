// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded public XMLTV ingestion with Lume first and EPGShare IT1 as a supplement.
mod epgshare;
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
    extended: crate::public_guide::PublicGuide,
    supplement: Arc<epgshare::EpgShare>,
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
            .user_agent(format!("WLTV {}", crate::providers::USER_AGENT))
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
        let service = Arc::new(Self {
            catalog: catalog.clone(),
            snapshot: RwLock::new(snapshot),
            loading: AtomicBool::new(false),
            attempted: AtomicU64::new(0),
            cache,
            client,
            extended: crate::public_guide::PublicGuide::new(directory),
            supplement: epgshare::EpgShare::new(directory)?,
        });
        catalog
            .personal
            .attach_public_guide(Arc::downgrade(&service));
        Ok(service)
    }

    pub fn snapshot(&self) -> GuideSnapshot {
        let mut snapshot = self.snapshot.read().expect("guide lock").clone();
        snapshot.loading = self.loading.load(Ordering::Relaxed);
        // Artwork supplied by the catalogue remains usable while the guide is
        // unavailable. Regional editions share artwork, never their schedules.
        for channel in self.catalog.snapshot().channels {
            if let Some(url) = logo_candidates(&snapshot, &channel).into_iter().next() {
                snapshot.logos.insert(channel.id, url);
            }
        }
        snapshot
    }

    pub fn refresh(self: &Arc<Self>, force: bool) {
        // A pause or failure at the Lume service must not block the independent feed.
        self.supplement.refresh(force);
        let now = now_ms();
        if now < self.snapshot.read().expect("guide lock").next_retry_at {
            return;
        }
        let signature = self.catalog.guide_signature();
        let fresh = {
            let snapshot = self.snapshot.read().expect("guide lock");
            now.saturating_sub(snapshot.updated_at) < 4 * HOUR
                && snapshot.catalog_signature == signature
                && self.extended.ready()
                && now.saturating_sub(self.extended.info().updated_at) < 4 * HOUR
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
        self.fetch_from(FEED, now_ms())
    }

    fn fetch_from(&self, endpoint: &str, at: u64) -> Result<GuideSnapshot> {
        let response = self
            .client
            .get(endpoint)
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
        self.parse_complete(xml, at)
    }

    fn parse_complete<R: BufRead>(&self, input: R, at: u64) -> Result<GuideSnapshot> {
        let mut writer = self.extended.begin(at)?;
        let snapshot = parse_feed(
            input,
            at,
            &self.catalog.snapshot().channels,
            Some(&mut writer),
        )?;
        self.extended.publish(writer)?;
        Ok(snapshot)
    }

    /// Reuse an already downloaded public XMLTV cache without another HTTP request.
    pub fn import_xml<R: BufRead>(&self, input: R, at: u64) -> Result<serde_json::Value> {
        let mut writer = self.extended.begin(at)?;
        let mut snapshot = parse_feed(
            BufReader::new(LimitedRead::new(input, 350 * 1024 * 1024)),
            now_ms(),
            &self.catalog.snapshot().channels,
            Some(&mut writer),
        )?;
        snapshot.updated_at = at;
        self.extended.publish(writer)?;
        let mut current = self.snapshot.write().expect("guide lock");
        if at >= current.updated_at {
            let bytes = serde_json::to_vec(&snapshot)?;
            let temporary = self.cache.with_extension("json.tmp");
            fs::write(&temporary, bytes)?;
            fs::rename(temporary, &self.cache)?;
            *current = snapshot;
        }
        drop(current);
        Ok(self.public_status())
    }

    /// Seed the independent supplement from an already downloaded public XMLTV file.
    pub fn import_epgshare_xml<R: BufRead>(&self, input: R, at: u64) -> Result<serde_json::Value> {
        self.supplement.import_xml(input, at)
    }

    pub(crate) fn metadata_revision(&self) -> String {
        let info = self.extended.info();
        let snapshot = self.snapshot.read().expect("guide lock");
        let extra = self.supplement.index.info();
        format!(
            "{}:{}:{}:{}:{}:{}",
            info.version,
            info.updated_at,
            snapshot.updated_at,
            self.catalog.snapshot().status.revision,
            extra.version,
            extra.updated_at,
        )
    }
    pub(crate) fn public_status(&self) -> serde_json::Value {
        let info = self.extended.info();
        let snapshot = self.snapshot.read().expect("guide lock");
        let supplement = self.supplement.status();
        let updated = if info.version > 0 {
            info.updated_at
        } else {
            snapshot.updated_at
        };
        serde_json::json!({
            "origin":"lume", "updated_at":updated.max(supplement["updated_at"].as_u64().unwrap_or(0)),
            "loading":self.loading.load(Ordering::Relaxed) || supplement["loading"] == true,
            "error":snapshot.error.clone().unwrap_or_default(),"next_retry_at":snapshot.next_retry_at,
            "matched":0,"programmes":if info.version>0 {info.programmes}else{snapshot.programmes.values().map(Vec::len).sum()},
            "channel_count":if info.version>0 {info.channels}else{snapshot.programmes.len()},
            "supplement":supplement
        })
    }
    pub(crate) fn lookup(
        &self,
        identities: &[crate::public_guide::Identity],
    ) -> Vec<Option<crate::public_guide::Match>> {
        let at = now_ms();
        let probes = self.extended.lookup(identities, at);
        let mut result = probes.iter().map(|p| p.data.clone()).collect::<Vec<_>>();
        let catalog = self.catalog.snapshot();
        let snapshot = self.snapshot.read().expect("guide lock");
        for ((identity, found), probe) in identities.iter().zip(&mut result).zip(&probes) {
            if probe.known && found.is_none() {
                continue;
            }
            if let Some(channel) = builtin_match(identity, &catalog.channels) {
                let logo = logo_candidates(&snapshot, channel)
                    .into_iter()
                    .next()
                    .unwrap_or_default();
                if let Some(found) = found {
                    if found.logo.is_empty() {
                        found.logo = logo;
                    }
                } else {
                    let list = snapshot
                        .programmes
                        .get(&channel.id)
                        .map(Vec::as_slice)
                        .unwrap_or(&[]);
                    *found = Some(crate::public_guide::Match {
                        logo,
                        now: list.iter().find(|p| p.start <= at && p.end > at).cloned(),
                        next: list.iter().find(|p| p.start > at).cloned(),
                        has_programmes: list.iter().any(|p| p.end > at.saturating_sub(6 * HOUR)),
                        origin: "lume",
                    });
                }
            }
        }
        drop(snapshot);
        let extra = self.supplement.index.lookup(identities, at);
        for ((primary, probe), secondary) in result.iter_mut().zip(&probes).zip(extra) {
            // Conflicting Lume identities do not become a weaker name match elsewhere.
            if probe.known && probe.data.is_none() {
                continue;
            }
            let Some(secondary) = secondary.data else {
                continue;
            };
            let primary = primary.get_or_insert_with(Default::default);
            if !primary.has_programmes && secondary.has_programmes {
                primary.now = secondary.now;
                primary.next = secondary.next;
                primary.has_programmes = true;
                primary.origin = "epgshare";
            }
            if primary.logo.is_empty() {
                primary.logo = secondary.logo;
            }
        }
        result
    }
    pub(crate) fn public_window(
        &self,
        identity: &crate::public_guide::Identity,
        from: u64,
        until: u64,
        text: &str,
        offset: usize,
    ) -> Result<(Vec<Programme>, usize, &'static str)> {
        if self.lookup(std::slice::from_ref(identity))[0]
            .as_ref()
            .is_some_and(|m| m.has_programmes && m.origin == "epgshare")
        {
            let (rows, total) = self
                .supplement
                .index
                .window(identity, from, until, text, offset)?;
            return Ok((rows, total, "epgshare"));
        }
        if self
            .extended
            .lookup(std::slice::from_ref(identity), now_ms())[0]
            .known
        {
            let (rows, total) = self.extended.window(identity, from, until, text, offset)?;
            return Ok((rows, total, "lume"));
        }
        let catalog = self.catalog.snapshot();
        let Some(channel) = builtin_match(identity, &catalog.channels) else {
            return Ok((vec![], 0, "lume"));
        };
        let snapshot = self.snapshot.read().expect("guide lock");
        let text = text.chars().take(300).collect::<String>().to_lowercase();
        let list = snapshot
            .programmes
            .get(&channel.id)
            .into_iter()
            .flatten()
            .filter(|p| {
                if text.is_empty() {
                    p.end > from && p.start < until.min(from.saturating_add(8 * 86_400_000))
                } else {
                    p.title.to_lowercase().contains(&text)
                        || p.description.to_lowercase().contains(&text)
                }
            })
            .collect::<Vec<_>>();
        Ok((
            list.iter()
                .skip(offset)
                .take(50)
                .map(|p| (*p).clone())
                .collect(),
            list.len(),
            "lume",
        ))
    }
    pub(crate) fn public_logo_candidates(
        &self,
        identity: &crate::public_guide::Identity,
    ) -> Vec<String> {
        let mut probes = self
            .extended
            .lookup(std::slice::from_ref(identity), now_ms());
        let probe = probes.remove(0);
        if probe.known && probe.data.is_none() {
            return vec![];
        }
        let mut urls = Vec::new();
        if let Some(channel) = builtin_match(identity, &self.catalog.snapshot().channels) {
            urls.extend(logo_candidates(
                &self.snapshot.read().expect("guide lock"),
                channel,
            ));
        }
        if let Some(data) = probe.data {
            if !data.logo.is_empty() && !urls.contains(&data.logo) {
                urls.push(data.logo);
            }
        }
        // A declared public Lume logo stays authoritative even if fetching it fails.
        if urls.is_empty() {
            if let Some(data) = self
                .supplement
                .index
                .lookup(std::slice::from_ref(identity), now_ms())
                .into_iter()
                .next()
                .and_then(|p| p.data)
            {
                if !data.logo.is_empty() {
                    urls.push(data.logo);
                }
            }
        }
        urls
    }

    pub fn logo_candidates(&self, id: &str) -> Vec<String> {
        let Some(channel) = self.catalog.channel(id) else {
            return Vec::new();
        };
        logo_candidates(&self.snapshot.read().expect("guide lock"), &channel)
    }
}

fn logo_candidates(snapshot: &GuideSnapshot, channel: &CatalogChannel) -> Vec<String> {
    let mut candidates = vec![channel.logo.clone()];
    candidates.extend(snapshot.logos.get(&channel.id).cloned());
    if canonical_epg(&channel.epg_id).starts_with("rai3tgr") {
        candidates.extend(snapshot.logos.get("rai3").cloned());
    }
    let mut result = Vec::new();
    for url in candidates {
        if allowed_logo(&url) && !result.contains(&url) {
            result.push(url);
        }
    }
    result
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
    if value.len() > 2048
        || !["http", "https"].contains(&url.scheme())
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.port().is_some()
    {
        return false;
    }
    if epg_logo_host(&url) {
        return url.query().is_none()
            && url.path().starts_with("/icn/icons/")
            && url.path().ends_with(".png");
    }
    crate::network::public_host(&url)
        && !url.host_str().is_some_and(|host| {
            [".test", ".invalid"]
                .iter()
                .any(|suffix| host.ends_with(suffix))
        })
        && (url.scheme() == "https" || url.host_str() == Some("www.man-ga.it"))
}

fn epg_logo_host(url: &Url) -> bool {
    matches!(
        url.host_str(),
        Some("epg-guide.com" | "www.epg-guide.com" | "116.202.210.205")
    )
}

pub(crate) fn allowed_logo_redirect(from: &Url, to: &Url) -> bool {
    if !allowed_logo(from.as_str()) || !allowed_logo(to.as_str()) {
        return false;
    }
    if epg_logo_host(from) {
        return epg_logo_host(to);
    }
    from.host_str()
        .unwrap_or_default()
        .trim_start_matches("www.")
        == to.host_str().unwrap_or_default().trim_start_matches("www.")
        && (from.scheme() != "https" || to.scheme() == "https")
}

pub(crate) fn canonical(value: &str) -> String {
    match canonical_epg(value).as_str() {
        "rai3tgr" => "rai3tgrveneto".into(),
        value => value.into(),
    }
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
    parse_feed(input, now, channels, None)
}
fn parse_feed<R: BufRead>(
    input: R,
    now: u64,
    channels: &[CatalogChannel],
    mut extended: Option<&mut crate::public_guide::Writer>,
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
    let mut raw_channel = String::new();
    let mut channel_names = Vec::new();
    let mut channel_logo = String::new();
    let mut programme_id = String::new();
    let mut channel_name = String::new();
    let mut depth = 0usize;
    let mut root = false;
    let mut declarations = 0usize;
    loop {
        let event = reader.read_event_into(&mut buffer).context("guide_xml")?;
        if event.as_ref().len() > 128 * 1024 {
            bail!("guide_declaration_limit");
        }
        match &event {
            Event::Start(e) => {
                depth += 1;
                if depth > 32 {
                    bail!("guide_depth_limit");
                }
                if depth == 1 {
                    if root || e.name().as_ref() != b"tv" {
                        bail!("guide_root");
                    }
                    root = true;
                }
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            Event::Eof if !root || depth != 0 => bail!("guide_truncated"),
            _ => {}
        }
        match event {
            Event::Start(e) if e.name().as_ref() == b"channel" => {
                raw_channel = attribute(&e, b"id");
                channel_names.clear();
                channel_logo.clear();
                declarations += 1;
                if declarations > 20_000 {
                    bail!("guide_channel_limit");
                }
                logo_id = canonical(&attribute(&e, b"id"));
                if !mapping.contains_key(&logo_id) {
                    logo_id.clear();
                }
            }
            Event::Empty(e) if e.name().as_ref() == b"channel" => {
                declarations += 1;
                if declarations > 20_000 {
                    bail!("guide_channel_limit");
                }
                if let Some(index) = extended.as_deref_mut() {
                    index.channel(&attribute(&e, b"id"), &[], "")?;
                }
            }
            Event::Empty(e) | Event::Start(e)
                if e.name().as_ref() == b"icon" && !raw_channel.is_empty() =>
            {
                let url = attribute(&e, b"src");
                if allowed_logo(&url) {
                    if channel_logo.is_empty() {
                        channel_logo = url.clone();
                    }
                    for id in mapping.get(&logo_id).into_iter().flatten() {
                        result
                            .logos
                            .entry(id.clone())
                            .or_insert_with(|| url.clone());
                    }
                }
            }
            Event::Start(e) if e.name().as_ref() == b"programme" => {
                programme_id = attribute(&e, b"channel");
                id = canonical(&programme_id);
                programme = Programme {
                    start: xmltv_timestamp(&attribute(&e, b"start")).unwrap_or(0),
                    end: xmltv_timestamp(&attribute(&e, b"stop")).unwrap_or(0),
                    ..Default::default()
                };
                if extended.is_none() && !mapping.contains_key(&id)
                    || programme.end <= now.saturating_sub(6 * HOUR)
                    || programme.start >= now + 48 * HOUR
                {
                    id.clear();
                    programme_id.clear();
                }
            }
            Event::Start(e) if !raw_channel.is_empty() && e.name().as_ref() == b"display-name" => {
                field = "display-name".into();
                channel_name.clear();
            }
            Event::Start(e)
                if !id.is_empty()
                    && [b"title".as_slice(), b"desc", b"category"].contains(&e.name().as_ref()) =>
            {
                field = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                italian = attribute(&e, b"lang") == "it";
            }
            Event::Text(e) if !raw_channel.is_empty() && field == "display-name" => {
                channel_name.extend(
                    e.unescape()
                        .context("guide_name")?
                        .chars()
                        .take(300usize.saturating_sub(channel_name.chars().count())),
                );
            }
            Event::CData(e) if !raw_channel.is_empty() && field == "display-name" => {
                channel_name.extend(
                    String::from_utf8_lossy(&e)
                        .chars()
                        .take(300usize.saturating_sub(channel_name.chars().count())),
                );
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
            Event::End(e) if e.name().as_ref() == b"display-name" && !raw_channel.is_empty() => {
                if channel_names.len() < 16 && !channel_name.trim().is_empty() {
                    channel_names.push(channel_name.trim().into());
                }
                field.clear();
            }
            Event::End(e) if e.name().as_ref() == b"channel" => {
                if let Some(index) = extended.as_deref_mut() {
                    index.channel(&raw_channel, &channel_names, &channel_logo)?;
                }
                logo_id.clear();
                raw_channel.clear();
                field.clear();
            }
            Event::End(e) if e.name().as_ref() == b"programme" => {
                if !id.is_empty()
                    && programme.end > programme.start
                    && programme.start > 0
                    && !programme.title.trim().is_empty()
                {
                    if let Some(index) = extended.as_deref_mut() {
                        index.programme(&programme_id, &programme)?;
                    }
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
    if result.programmes.is_empty() && extended.is_none() {
        bail!("guide_empty");
    }
    Ok(result)
}

fn builtin_match<'a>(
    identity: &crate::public_guide::Identity,
    channels: &'a [CatalogChannel],
) -> Option<&'a CatalogChannel> {
    let named = channels
        .iter()
        .filter(|c| {
            identity
                .names
                .iter()
                .any(|n| crate::public_guide::name_key(n) == crate::public_guide::name_key(&c.name))
        })
        .collect::<Vec<_>>();
    let mut ids = channels
        .iter()
        .filter(|c| {
            !identity.xml_id.is_empty() && canonical(&identity.xml_id) == canonical(&c.epg_id)
        })
        .collect::<Vec<_>>();
    if ids.is_empty() {
        ids = named.clone();
    }
    if ids.len() != 1 || !named.is_empty() && (named.len() != 1 || named[0].id != ids[0].id) {
        return None;
    }
    let target = ids[0];
    crate::public_guide::compatible_identity(
        identity,
        &target.epg_id,
        std::slice::from_ref(&target.name),
    )
    .then_some(target)
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
    fn epgshare_is_last_priority_and_programmes_and_logos_are_independent() {
        use crate::public_guide::Identity;
        let root =
            std::env::temp_dir().join(format!("lume-guide-priority-{}", uuid::Uuid::new_v4()));
        let service = GuideService::new(&root).unwrap();
        let now = chrono::Utc::now();
        let start = (now - chrono::Duration::minutes(10)).format("%Y%m%d%H%M%S +0000");
        let stop = (now + chrono::Duration::minutes(50)).format("%Y%m%d%H%M%S +0000");
        let primary = format!(
            r#"<tv>
          <channel id="Priority.it"><display-name>Prioritario</display-name><icon src="https://images.example.org/lume.png"/></channel>
          <channel id="OnlyLogo.it"><display-name>Solo Logo</display-name><icon src="https://images.example.org/lume-logo.png"/></channel>
          <channel id="OnlyGuide.it"><display-name>Solo Guida</display-name></channel>
          <programme channel="Priority.it" start="{start}" stop="{stop}"><title>Palinsesto Lume</title></programme>
          <programme channel="OnlyGuide.it" start="{start}" stop="{stop}"><title>Guida Lume senza logo</title></programme></tv>"#
        );
        service
            .import_xml(std::io::Cursor::new(primary), now_ms())
            .unwrap();
        let mut secondary = "<tv>".to_owned();
        for (id, name) in [
            ("Priority.it", "Prioritario"),
            ("OnlyLogo.it", "Solo Logo"),
            ("OnlyGuide.it", "Solo Guida"),
            ("Sky.Uno.it", "Sky Uno"),
        ] {
            secondary.push_str(&format!(r#"<channel id="{id}"><display-name>{name}</display-name><icon src="https://guidatv.sky.it/logo/extra.png"/></channel><programme channel="{id}" start="{start}" stop="{stop}"><title>Palinsesto EPGShare</title></programme>"#));
        }
        secondary.push_str("</tv>");
        let revision = service.metadata_revision();
        service
            .import_epgshare_xml(std::io::Cursor::new(secondary), now_ms())
            .unwrap();
        assert_ne!(service.metadata_revision(), revision);
        let identities =
            ["Priority.it", "OnlyLogo.it", "OnlyGuide.it", "Sky.Uno.it"].map(|id| Identity {
                xml_id: id.into(),
                names: vec![],
            });
        let matches = service.lookup(&identities);
        for (index, title, origin, logo) in [
            (
                0,
                "Palinsesto Lume",
                "lume",
                "https://images.example.org/lume.png",
            ),
            (
                1,
                "Palinsesto EPGShare",
                "epgshare",
                "https://images.example.org/lume-logo.png",
            ),
            (
                2,
                "Guida Lume senza logo",
                "lume",
                "https://guidatv.sky.it/logo/extra.png",
            ),
            (
                3,
                "Palinsesto EPGShare",
                "epgshare",
                "https://guidatv.sky.it/logo/extra.png",
            ),
        ] {
            let data = matches[index].as_ref().unwrap();
            assert_eq!(data.now.as_ref().unwrap().title, title);
            assert_eq!(data.origin, origin);
            assert_eq!(
                service.public_logo_candidates(&identities[index]),
                vec![logo]
            );
            let (rows, total, selected) = service
                .public_window(
                    &identities[index],
                    now_ms() - 3_600_000,
                    now_ms() + 3_600_000,
                    "",
                    0,
                )
                .unwrap();
            assert_eq!(total, 1);
            assert_eq!(rows[0].title, title);
            assert_eq!(selected, origin);
        }
        // A search/date without Lume results must not mix in the secondary schedule.
        assert_eq!(
            service
                .public_window(&identities[0], 0, 1, "EPGShare", 0)
                .unwrap()
                .1,
            0
        );
        assert_eq!(service.snapshot().programmes.len(), 0);
        drop(service);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn conflicting_primary_identity_stays_excluded_with_a_supplement() {
        let root =
            std::env::temp_dir().join(format!("lume-guide-conflict-{}", uuid::Uuid::new_v4()));
        let service = GuideService::new(&root).unwrap();
        let now = chrono::Utc::now();
        let start = (now - chrono::Duration::minutes(10)).format("%Y%m%d%H%M%S +0000");
        let stop = (now + chrono::Duration::minutes(50)).format("%Y%m%d%H%M%S +0000");
        let primary = format!(
            r#"<tv><channel id="First.it"><display-name>Primo</display-name></channel><channel id="Second.it"><display-name>Secondo</display-name></channel><programme channel="First.it" start="{start}" stop="{stop}"><title>Primo programma</title></programme></tv>"#
        );
        service
            .import_xml(std::io::Cursor::new(primary), now_ms())
            .unwrap();
        let extra = format!(
            r#"<tv><channel id="First.it"><display-name>Secondo</display-name><icon src="https://guidatv.sky.it/logo/extra.png"/></channel><programme channel="First.it" start="{start}" stop="{stop}"><title>Non associare</title></programme></tv>"#
        );
        service
            .import_epgshare_xml(std::io::Cursor::new(extra), now_ms())
            .unwrap();
        let identity = crate::public_guide::Identity {
            xml_id: "First.it".into(),
            names: vec!["Secondo".into()],
        };
        assert!(service.lookup(std::slice::from_ref(&identity))[0].is_none());
        assert!(service.public_logo_candidates(&identity).is_empty());
        drop(service);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn full_feed_preserves_extra_channels_and_rejects_invalid_replacements_atomically() {
        let root = std::env::temp_dir().join(format!("lume-full-feed-{}", uuid::Uuid::new_v4()));
        let service = GuideService::new(&root).unwrap();
        let at = now_ms();
        let now = chrono::Utc::now();
        let start = (now - chrono::Duration::minutes(10)).format("%Y%m%d%H%M%S +0000");
        let stop = (now + chrono::Duration::minutes(10)).format("%Y%m%d%H%M%S +0000");
        let xml = format!(
            r#"<tv><channel id="Rai1.it"><display-name>Rai 1</display-name></channel><channel id="Extra.it"><display-name>Esteso <![CDATA[È]]> Uno</display-name><icon src="https://images.example.org/extra.png"></icon></channel><programme channel="Rai1.it" start="{start}" stop="{stop}"><title>Programma Rai</title></programme><programme channel="Extra.it" start="{start}" stop="{stop}"><title>Programma esteso</title></programme></tv>"#
        );
        service.import_xml(std::io::Cursor::new(xml), at).unwrap();
        assert_eq!(service.extended.info().channels, 2);
        assert_eq!(service.snapshot().programmes.len(), 1);
        let identity = crate::public_guide::Identity {
            xml_id: String::new(),
            names: vec!["Esteso E Uno HD".into()],
        };
        assert_eq!(
            service.lookup(std::slice::from_ref(&identity))[0]
                .as_ref()
                .unwrap()
                .now
                .as_ref()
                .unwrap()
                .title,
            "Programma esteso"
        );
        assert!(!serde_json::to_string(&service.snapshot())
            .unwrap()
            .contains("Programma esteso"));
        for invalid in [
            "<tv><channel id='broken'>",
            "<!DOCTYPE tv [<!ENTITY x SYSTEM 'file:///etc/passwd'>]><tv></tv>",
            "<html><channel id='fake'/></html>",
        ] {
            assert!(service
                .import_xml(std::io::Cursor::new(invalid), at + 1)
                .is_err());
            assert_eq!(service.extended.info().updated_at, at);
        }
        assert_eq!(service.extended.info().programmes, 2);
        assert!(fs::read_dir(&root).unwrap().flatten().all(|f| !f
            .file_name()
            .to_string_lossy()
            .starts_with(".public-guide-")));
        drop(service);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn catalogue_and_regional_artwork_remain_available_during_guide_backoff() {
        let root = std::env::temp_dir().join(format!("lume-guide-logos-{}", uuid::Uuid::new_v4()));
        let service = GuideService::new(&root).unwrap();
        let catalogue_logo = catalog().iter().find(|c| !c.logo.is_empty()).unwrap();
        let regional = catalog()
            .iter()
            .find(|c| c.epg_id == "rai3tgrpiemonte")
            .unwrap();
        let retry = now_ms() + 60_000;
        *service.snapshot.write().unwrap() = GuideSnapshot {
            logos: BTreeMap::from([(
                "rai3".into(),
                "https://epg-guide.com/icn/icons/rai3.png".into(),
            )]),
            next_retry_at: retry,
            error: Some("guide_rate_limit".into()),
            ..Default::default()
        };
        service.refresh(true);
        assert_eq!(service.attempted.load(Ordering::Relaxed), 0);
        let snapshot = service.snapshot();
        assert_eq!(snapshot.logos[&catalogue_logo.id], catalogue_logo.logo);
        assert_eq!(snapshot.logos[&regional.id], snapshot.logos["rai3"]);
        assert!(snapshot.programmes.is_empty());
        assert_eq!(snapshot.next_retry_at, retry);
        assert_eq!(snapshot.error.as_deref(), Some("guide_rate_limit"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn observed_tgr_feed_alias_maps_only_to_veneto_without_copying_other_schedules() {
        let xml = br#"<tv><channel id="Rai3TGR.it"><icon src="https://epg-guide.com/icn/icons/rai3tgr.png"/></channel><programme channel="Rai3TGR.it" start="20261108080000 +0100" stop="20261108100000 +0100"><title>Edizione regionale</title></programme></tv>"#;
        let result = parse(
            xml.as_slice(),
            xmltv_timestamp("20261108090000 +0100").unwrap(),
        )
        .unwrap();
        let veneto = catalog()
            .iter()
            .find(|c| c.epg_id == "rai3tgrveneto")
            .unwrap();
        assert_eq!(result.programmes.len(), 1);
        assert_eq!(result.programmes[&veneto.id][0].title, "Edizione regionale");
        assert!(result.logos.contains_key(&veneto.id));
    }

    #[test]
    fn public_catalogue_logos_and_restricted_redirects_follow_android_policy() {
        for value in [
            "https://upload.wikimedia.org/catalogue/logo.png",
            "https://business24tv.it/logo.jpg",
            "http://www.man-ga.it/logo.gif",
            "http://116.202.210.205/icn/icons/rai1.png",
        ] {
            assert!(allowed_logo(value), "{value}");
        }
        for value in [
            "http://untrusted.example/logo.png",
            "https://epg-guide.com/private.png",
            "https://epg-guide.com/icn/icons/rai1.png?token=test",
            "https://user:password@public.example/logo.png",
            "https://127.0.0.1/logo.png",
            "https://internal.local/logo.png",
            "https://public.invalid/logo.png",
            "https://public.example:8443/logo.png",
        ] {
            assert!(!allowed_logo(value), "{value}");
        }
        let valid =
            |from, to| allowed_logo_redirect(&Url::parse(from).unwrap(), &Url::parse(to).unwrap());
        assert!(valid(
            "http://116.202.210.205/icn/icons/a.png",
            "https://epg-guide.com/icn/icons/b.png"
        ));
        assert!(valid(
            "https://business24tv.it/logo.jpg",
            "https://www.business24tv.it/new.png"
        ));
        assert!(!valid(
            "https://business24tv.it/logo.jpg",
            "https://other.example/new.png"
        ));
        assert!(!valid(
            "https://www.man-ga.it/logo.gif",
            "http://www.man-ga.it/logo.gif"
        ));
        assert!(!valid(
            "https://epg-guide.com/icn/icons/a.png",
            "https://other.example/logo.png"
        ));
    }

    #[test]
    fn public_feed_uses_android_client_identity_and_imports_programmes_and_artwork() {
        use base64::Engine;
        let root = std::env::temp_dir().join(format!("lume-guide-http-{}", uuid::Uuid::new_v4()));
        let service = GuideService::new(&root).unwrap();
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(include_str!("../tests/fixtures/guide-public.xml.xz.base64").trim())
            .unwrap();
        let worker = std::thread::spawn(move || {
            for status in [429, 200] {
                let request = server
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap();
                let ua = request
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("User-Agent"))
                    .unwrap()
                    .value
                    .as_str();
                assert_eq!(ua, format!("WLTV {}", crate::providers::USER_AGENT));
                let response = if status == 429 {
                    tiny_http::Response::from_data(Vec::new())
                        .with_status_code(429)
                        .with_header(tiny_http::Header::from_bytes("Retry-After", "120").unwrap())
                } else {
                    tiny_http::Response::from_data(bytes.clone())
                };
                request.respond(response).unwrap();
            }
        });
        let endpoint = format!("http://{address}/guide.xz");
        let at = xmltv_timestamp("20261108083000 +0100").unwrap();
        let error = service.fetch_from(&endpoint, at).unwrap_err();
        assert_eq!(error.downcast_ref::<GuideRateLimit>().unwrap().seconds, 120);
        let result = service.fetch_from(&endpoint, at).unwrap();
        let id = catalog()
            .iter()
            .find(|c| c.epg_id == "tv8")
            .unwrap()
            .id
            .clone();
        assert_eq!(result.programmes[&id][0].title, "Guida di prova");
        assert!(result.logos[&id].ends_with("/tv8.png"));
        worker.join().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
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
