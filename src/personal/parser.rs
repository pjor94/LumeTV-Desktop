// SPDX-License-Identifier: GPL-3.0-or-later
use super::{
    network::{validate_url, Context, Resource},
    Entry, Headers, ImportReport, Metadata, Settings, MAX_BYTES, MAX_ENTRIES,
};
use anyhow::{anyhow, bail, Result};
use reqwest::{Method, Url};
use std::{
    collections::{BTreeMap, HashSet},
    io::{self, BufRead, BufReader, Read},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, OnceLock,
    },
};

fn attributes(text: &str) -> BTreeMap<String, String> {
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    PATTERN
        .get_or_init(|| regex::Regex::new(r#"([\w-]+)\s*=\s*(?:"([^"]*)"|'([^']*)')"#).unwrap())
        .captures_iter(text)
        .map(|c| {
            (
                c[1].to_lowercase(),
                c.get(2).or_else(|| c.get(3)).unwrap().as_str().into(),
            )
        })
        .collect()
}
pub fn resolve(base: Option<&str>, value: &str, allow_http: bool) -> Result<String> {
    let url = match Url::parse(value) {
        Ok(url) => url,
        Err(_) => {
            Url::parse(base.ok_or_else(|| anyhow!("Un file locale richiede indirizzi assoluti."))?)?
                .join(value)?
        }
    };
    Ok(validate_url(url.as_str(), allow_http)?.into())
}

struct Counted {
    inner: Box<dyn Read + Send>,
    bytes: Arc<AtomicU64>,
}
impl Read for Counted {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buffer)?;
        if self.bytes.fetch_add(n as u64, Ordering::Relaxed) + n as u64 > MAX_BYTES {
            return Err(io::Error::other("La lista supera 256 MiB."));
        }
        Ok(n)
    }
}
struct Latin1 {
    inner: Box<dyn Read + Send>,
    pending: std::collections::VecDeque<u8>,
}
impl Read for Latin1 {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        if self.pending.is_empty() {
            let mut input = [0u8; 4096];
            let n = self.inner.read(&mut input)?;
            for &b in &input[..n] {
                if b < 128 {
                    self.pending.push_back(b);
                } else {
                    self.pending.push_back(0xc0 | (b >> 6));
                    self.pending.push_back(0x80 | (b & 0x3f));
                }
            }
        }
        let n = out.len().min(self.pending.len());
        for byte in &mut out[..n] {
            *byte = self.pending.pop_front().unwrap();
        }
        Ok(n)
    }
}
pub fn response_reader(response: super::network::NetworkResponse) -> Box<dyn Read + Send> {
    match response
        .headers()
        .get("Content-Encoding")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "gzip" => Box::new(flate2::read::GzDecoder::new(response)),
        "deflate" => Box::new(flate2::read::ZlibDecoder::new(response)),
        _ => Box::new(response),
    }
}

#[derive(Default)]
struct Pending {
    name: String,
    duration: f64,
    attrs: BTreeMap<String, String>,
    properties: BTreeMap<String, String>,
    headers: Headers,
    explicit_group: bool,
}
pub struct Scanner<'a> {
    pub settings: &'a Settings,
    pub context: Arc<Context>,
    pub report: ImportReport,
    pub bytes: Arc<AtomicU64>,
    pub sink: &'a mut dyn FnMut(&Entry) -> Result<bool>,
    pub check: &'a dyn Fn() -> Result<()>,
    pub progress: &'a dyn Fn(u64, usize),
    visited: HashSet<String>,
}
impl<'a> Scanner<'a> {
    pub fn new(
        settings: &'a Settings,
        context: Arc<Context>,
        sink: &'a mut dyn FnMut(&Entry) -> Result<bool>,
        check: &'a dyn Fn() -> Result<()>,
        progress: &'a dyn Fn(u64, usize),
    ) -> Self {
        Self {
            settings,
            context,
            report: ImportReport::default(),
            bytes: Arc::new(AtomicU64::new(0)),
            sink,
            check,
            progress,
            visited: HashSet::new(),
        }
    }
    pub fn scan(&mut self, input: Box<dyn Read + Send>, base: Option<&str>) -> Result<()> {
        self.document(input, base, 0, BTreeMap::new())?;
        self.report.bytes = self.bytes.load(Ordering::Relaxed);
        Ok(())
    }
    fn document(
        &mut self,
        input: Box<dyn Read + Send>,
        base: Option<&str>,
        depth: usize,
        inherited: BTreeMap<String, String>,
    ) -> Result<()> {
        if depth > 3 || self.visited.len() >= 16 {
            bail!("Troppe liste incluse: massimo 16 documenti e tre livelli.");
        }
        if let Some(base) = base {
            if !self.visited.insert(base.into()) {
                bail!("La lista contiene un riferimento ciclico.");
            }
        } else {
            self.visited.insert("file".into());
        }
        let counted: Box<dyn Read + Send> = Box::new(Counted {
            inner: input,
            bytes: self.bytes.clone(),
        });
        let decoded: Box<dyn Read + Send> = if self.settings.encoding == "latin-1" {
            Box::new(Latin1 {
                inner: counted,
                pending: Default::default(),
            })
        } else {
            Box::new(
                encoding_rs_io::DecodeReaderBytesBuilder::new()
                    .encoding(Some(if self.settings.encoding == "windows-1252" {
                        encoding_rs::WINDOWS_1252
                    } else {
                        encoding_rs::UTF_8
                    }))
                    .utf8_passthru(true)
                    .build(counted),
            )
        };
        let mut reader = BufReader::new(decoded);
        let mut line = String::new();
        let mut first = true;
        let mut global = inherited;
        let mut group = String::new();
        let mut pending: Option<Pending> = None;
        loop {
            (self.check)()?;
            self.context.check()?;
            line.clear();
            let n = reader
                .by_ref()
                .take(65537)
                .read_line(&mut line)
                .map_err(|_| anyhow!("La lista è incompleta o usa una codifica non valida."))?;
            if n == 0 {
                break;
            }
            if n > 65536 {
                bail!("La lista contiene una riga oltre 64 KiB.");
            }
            let text = line.trim().trim_start_matches('\u{feff}');
            if text.is_empty() {
                continue;
            }
            if text.contains('\u{fffd}') {
                bail!(
                    "La codifica della lista non è valida. Scegli UTF-8, Windows-1252 o Latin-1."
                );
            }
            if first {
                first = false;
                if self.settings.format != "simple" && !text.starts_with("#EXTM3U") {
                    bail!("La lista deve iniziare con #EXTM3U.");
                }
                if text.starts_with('<') {
                    bail!("Il documento ricevuto non è una lista M3U.");
                }
            }
            if self.settings.group_profile == "kodi" && text == "#EXT-X-PLAYLIST-TYPE:VOD" {
                global.insert("type".into(), "vod".into());
                continue;
            }
            if text.starts_with("#EXT-X-") {
                bail!("Un manifest HLS non può essere importato come catalogo.");
            }
            if text.starts_with("#EXTM3U") {
                let mut declared = attributes(text);
                for key in ["url-tvg", "x-tvg-url"] {
                    if let Some(value) = declared.get_mut(key) {
                        *value = value
                            .split([',', ';'])
                            .filter_map(|url| resolve(base, url.trim(), true).ok())
                            .collect::<Vec<_>>()
                            .join(",");
                    }
                }
                if let Some(value) = declared.get_mut("url-logo") {
                    if let Ok(url) = resolve(base, value, true) {
                        *value = url;
                    }
                }
                global.extend(declared);
                self.collect_guide(&global, base)?;
                if let Some(value) = global.get("refresh") {
                    self.report.refresh_minutes = interval(value).unwrap_or(0);
                }
                continue;
            }
            if let Some(value) = text.strip_prefix("#EXTINF:") {
                if pending.is_some() {
                    bail!("Una voce della lista è priva di indirizzo.");
                }
                let mut quote = None;
                let mut separator = None;
                for (i, c) in value.char_indices() {
                    if Some(c) == quote {
                        quote = None;
                    } else if quote.is_none() && ['\"', '\''].contains(&c) {
                        quote = Some(c);
                    } else if c == ',' && quote.is_none() {
                        separator = Some(i);
                        break;
                    }
                }
                let i = separator.ok_or_else(|| anyhow!("Voce EXTINF non valida."))?;
                let duration = value
                    .split_whitespace()
                    .next()
                    .unwrap_or("-1")
                    .split(',')
                    .next()
                    .unwrap_or("-1")
                    .parse::<f64>()
                    .unwrap_or(-1.0);
                let local = attributes(&value[..i]);
                let explicit_group = local.contains_key("group-title");
                let mut attrs = global.clone();
                attrs.extend(local);
                if !group.is_empty() && !attrs.contains_key("group-title") {
                    attrs.insert("group-title".into(), group.clone());
                }
                if self.settings.group_profile == "ott" {
                    group.clear();
                }
                pending = Some(Pending {
                    name: value[i + 1..].trim().into(),
                    duration,
                    attrs,
                    explicit_group,
                    ..Default::default()
                });
                continue;
            }
            if let Some(value) = text.strip_prefix("#EXTGRP:") {
                group = value.chars().take(2000).collect();
                if let Some(p) = pending.as_mut() {
                    p.attrs.insert("group-title".into(), group.clone());
                }
                continue;
            }
            if let Some(p) = pending.as_mut() {
                if let Some(value) = text.strip_prefix("#KODIPROP:") {
                    if let Some((key, value)) = value.split_once('=') {
                        if p.properties.len() >= 32
                            || value.len() > 32768
                            || p.properties.values().map(String::len).sum::<usize>() + value.len()
                                > 128 * 1024
                        {
                            bail!("Configurazione della voce troppo grande.");
                        }
                        p.properties
                            .insert(key.trim().to_lowercase(), value.trim().into());
                    }
                    continue;
                }
                if let Some(value) = text.strip_prefix("#EXTHTTP:") {
                    p.properties.insert("exthttp".into(), value.into());
                    continue;
                }
                if let Some(value) = text.strip_prefix("#EXTVLCOPT:") {
                    if let Some((key, value)) = value.split_once('=') {
                        let header = match key.to_lowercase().as_str() {
                            "http-user-agent" => Some("User-Agent"),
                            "http-referrer" | "http-referer" => Some("Referer"),
                            "http-origin" => Some("Origin"),
                            "http-cookie" => Some("Cookie"),
                            _ => None,
                        };
                        if let Some(header) = header {
                            p.headers.insert(header.into(), value.into());
                        }
                    }
                    continue;
                }
            }
            if text.starts_with('#') {
                continue;
            }
            self.report.scanned += 1;
            if self.report.scanned > MAX_ENTRIES {
                bail!("La lista supera 500.000 voci, comprese le liste incluse.");
            }
            let p = match pending.take() {
                Some(p) => p,
                None if self.settings.format == "simple" => Pending {
                    duration: -1.0,
                    attrs: global.clone(),
                    ..Default::default()
                },
                None => bail!("Una voce della lista è priva di EXTINF."),
            };
            if ["type", "content-type"].iter().any(|key| {
                p.attrs
                    .get(*key)
                    .is_some_and(|v| v.eq_ignore_ascii_case("playlist"))
            }) {
                let target = resolve(base, text, self.settings.allow_http)?;
                let response = self.context.request(
                    &target,
                    &target,
                    Resource::Playlist,
                    &Headers::new(),
                    Method::GET,
                    None,
                )?;
                if !response.status().is_success() {
                    bail!("Una lista inclusa non è disponibile. La copia precedente resta conservata.");
                }
                let final_url = response.url().to_string();
                self.document(
                    response_reader(response),
                    Some(&final_url),
                    depth + 1,
                    global.clone(),
                )?;
                continue;
            }
            if self.settings.group_profile == "ott"
                || self.settings.group_profile == "kodi" && p.explicit_group
            {
                group.clear();
            }
            match self.entry(p, text, base) {
                Ok(entry) => {
                    if (self.sink)(&entry)? {
                        self.report.count += 1;
                        if entry.content == "vod" {
                            self.report.vod += 1;
                        }
                        if entry.kind == crate::model::MediaKind::Radio {
                            self.report.radio += 1;
                        }
                        if self.report.sample.len() < 5 {
                            self.report.sample.push(entry.name.clone());
                        }
                        for url in &entry.metadata.guide_urls {
                            if !self.report.guide_urls.contains(url)
                                && self.report.guide_urls.len() < 16
                            {
                                self.report.guide_urls.push(url.clone());
                            }
                        }
                    } else {
                        self.report.duplicates += 1;
                    }
                }
                Err(error) => self.report.exclude(&error.to_string()),
            }
            if self.report.scanned.is_multiple_of(200) {
                (self.progress)(self.bytes.load(Ordering::Relaxed), self.report.scanned);
            }
        }
        if pending.is_some() {
            bail!("L’ultima voce della lista è incompleta.");
        }
        if first {
            bail!("La lista è vuota.");
        }
        Ok(())
    }
    fn collect_guide(
        &mut self,
        attrs: &BTreeMap<String, String>,
        base: Option<&str>,
    ) -> Result<()> {
        for key in ["url-tvg", "x-tvg-url"] {
            if let Some(value) = attrs.get(key) {
                for raw in value.split([',', ';']) {
                    if let Ok(url) = resolve(base, raw.trim(), true) {
                        if self.report.guide_urls.len() < 16
                            && !self.report.guide_urls.contains(&url)
                        {
                            self.report.guide_urls.push(url);
                        }
                    }
                }
            }
        }
        Ok(())
    }
    fn entry(&self, mut p: Pending, text: &str, base: Option<&str>) -> Result<Entry> {
        let (raw, pipe) = text.split_once('|').unwrap_or((text, ""));
        let route = super::localplay::read(raw);
        let url = if super::localplay::command(raw) {
            let route =
                route.ok_or_else(|| anyhow!("Il collegamento localplay non è supportato."))?;
            if !pipe.is_empty() {
                bail!("Il collegamento localplay non supporta parametri aggiuntivi.");
            }
            p.properties
                .insert("lumetv.localplay".into(), route.name.to_lowercase());
            format!("localplay={}", route.key)
        } else {
            resolve(base, raw, self.settings.allow_http)
                .map_err(|_| anyhow!("Indirizzo non supportato o consenso HTTP mancante."))?
        };
        if let Some(route) = route.filter(|_| p.name.is_empty()) {
            p.name = route.key.into();
        }
        if p.name.is_empty() {
            p.name = Url::parse(&url)?
                .path_segments()
                .and_then(|mut p| p.next_back())
                .filter(|s| !s.is_empty())
                .unwrap_or("Canale")
                .into();
        }
        if p.name.chars().count() > 300 || p.name.chars().any(char::is_control) {
            bail!("Nome del canale non valido.");
        }
        if !pipe.is_empty() {
            p.properties
                .insert("lumetv.pipe_headers".into(), pipe.into());
        }
        for (attr, header) in [
            ("http-user-agent", "User-Agent"),
            ("http-referrer", "Referer"),
        ] {
            if let Some(value) = p.attrs.get(attr) {
                p.headers
                    .entry(header.into())
                    .or_insert_with(|| value.clone());
            }
        }
        let groups = p
            .attrs
            .get("group-title")
            .map(|s| {
                s.split(';')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .take(20)
                    .map(|s| s.chars().take(100).collect())
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();
        let kind = if self.settings.kind == "radio"
            || p.attrs
                .get("radio")
                .is_some_and(|v| ["true", "1"].contains(&v.as_str()))
        {
            crate::model::MediaKind::Radio
        } else {
            crate::model::MediaKind::Tv
        };
        let content = if p.duration > 0.0
            || ["type", "content-type", "media-type"].iter().any(|key| {
                p.attrs.get(*key).is_some_and(|v| {
                    [
                        "vod", "video", "movie", "movies", "film", "films", "series", "episode",
                        "episodes", "tvshow",
                    ]
                    .contains(&v.to_lowercase().as_str())
                })
            }) {
            "vod"
        } else {
            "live"
        };
        let guide_urls = ["url-tvg", "x-tvg-url"]
            .iter()
            .filter_map(|k| p.attrs.get(*k))
            .flat_map(|s| s.split([',', ';']))
            .filter_map(|s| resolve(base, s.trim(), true).ok())
            .take(16)
            .collect();
        let logo = p.attrs.get("tvg-logo").map(String::as_str).unwrap_or("");
        let logo = if logo.is_empty() {
            String::new()
        } else if let Some(root) = p.attrs.get("url-logo") {
            let base_logo = resolve(base, root, true).ok();
            base_logo
                .and_then(|b| {
                    resolve(Some(&format!("{}/", b.trim_end_matches('/'))), logo, true).ok()
                })
                .unwrap_or_default()
        } else {
            resolve(base, logo, true).unwrap_or_default()
        };
        for key in [
            "poster",
            "poster-url",
            "tvg-poster",
            "cover",
            "cover-url",
            "backdrop",
            "backdrop-url",
            "fanart",
            "fanart-url",
            "tvg-background",
        ] {
            if let Some(value) = p.attrs.get_mut(key) {
                *value = resolve(base, value, true).unwrap_or_default();
            }
        }
        let number = p
            .attrs
            .get("tvg-chno")
            .or_else(|| p.attrs.get("ch-number"))
            .and_then(|s| s.parse::<u32>().ok())
            .filter(|n| *n <= 999999);
        let mut entry = Entry {
            id: format!("personal_{}", uuid::Uuid::new_v4().simple()),
            name: p.name,
            number,
            kind,
            content: content.into(),
            groups,
            logo,
            url,
            headers: p.headers,
            metadata: Metadata {
                duration: if p.duration.is_finite() {
                    p.duration
                } else {
                    0.0
                },
                tvg_id: p.attrs.get("tvg-id").cloned().unwrap_or_default(),
                tvg_name: p.attrs.get("tvg-name").cloned().unwrap_or_default(),
                guide_urls,
                shift_hours: p
                    .attrs
                    .get("tvg-shift")
                    .and_then(|s| s.parse::<f64>().ok())
                    .filter(|v| v.is_finite() && v.abs() <= 24.0)
                    .unwrap_or(0.0),
                attributes: p.attrs,
                properties: p.properties,
                catchup_id: String::new(),
            },
            position: self.report.count,
        };
        entry.content = super::video::classify(&entry, self.settings).into();
        super::localplay::configure(&entry, self.context.clone(), None)?;
        Ok(entry)
    }
}
pub fn interval(value: &str) -> Option<u32> {
    let value = value.trim().to_uppercase();
    let seconds = if let Some(v) = value.strip_prefix("PT") {
        if let Some(v) = v.strip_suffix('H') {
            v.parse::<f64>().ok()? * 3600.0
        } else if let Some(v) = v.strip_suffix('M') {
            v.parse::<f64>().ok()? * 60.0
        } else {
            v.strip_suffix('S')?.parse().ok()?
        }
    } else if let Some(v) = value.strip_prefix('P').and_then(|v| v.strip_suffix('D')) {
        v.parse::<f64>().ok()? * 86400.0
    } else {
        let (n, scale) = value.split_at(value.len().checked_sub(1)?);
        n.parse::<f64>().ok()?
            * match scale {
                "S" => 1.0,
                "M" => 60.0,
                "H" => 3600.0,
                "D" => 86400.0,
                _ => return None,
            }
    };
    if !seconds.is_finite() || seconds <= 0.0 {
        return None;
    }
    Some((seconds / 60.0).round().clamp(15.0, 10080.0) as u32)
}
