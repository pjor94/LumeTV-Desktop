// SPDX-License-Identifier: GPL-3.0-or-later
//! Personal library adapted from Lume TV Android 0.9.0 and its November 13
//! continuous-catalog work. Credentials never enter catalog display rows.
pub mod backup;
mod catchup;
pub(crate) mod drm;
mod guide;
mod hls;
mod localplay;
mod metadata;
pub(crate) mod network;
mod parser;
mod store;
pub mod video;
pub(crate) mod wire;

use crate::{catalog::CatalogChannel, model::ResolvedStream};
use anyhow::{bail, Result};
pub use network::{Context, Resource};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};
pub use store::{CatalogQuery, ImportConfig, Library};

pub const MAX_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 500_000;
pub const MAX_TOTAL: usize = 1_000_000;
pub const MAX_DATABASE: u64 = 1024 * 1024 * 1024;
pub const MAX_DISK: u64 = 2 * MAX_DATABASE;
pub type Headers = BTreeMap<String, String>;

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Access {
    pub headers: Headers,
    pub origins: Vec<String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct GuideSettings {
    pub enabled: bool,
    pub urls: Vec<String>,
    pub allow_http: bool,
    pub language: String,
    pub timezone: String,
    pub shift_minutes: i32,
    pub history_days: u32,
    pub future_hours: u32,
    pub mappings: BTreeMap<String, String>,
    pub archive_days: Option<u32>,
    pub archive_timezone: String,
    pub ongoing: bool,
    pub before_seconds: u32,
    pub after_seconds: u32,
}
impl Default for GuideSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            urls: vec![],
            allow_http: false,
            language: "it".into(),
            timezone: "Europe/Rome".into(),
            shift_minutes: 0,
            history_days: 7,
            future_hours: 48,
            mappings: BTreeMap::new(),
            archive_days: None,
            archive_timezone: "Europe/Rome".into(),
            ongoing: false,
            before_seconds: 0,
            after_seconds: 0,
        }
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub kind: String,
    pub allow_http: bool,
    pub allow_http_logos: bool,
    pub cookies: bool,
    pub encoding: String,
    pub format: String,
    pub group_profile: String,
    pub content: String,
    pub vod_detection: String,
    pub group_content: BTreeMap<String, String>,
    pub refresh_minutes: u32,
    pub access: BTreeMap<Resource, Access>,
    pub guide: GuideSettings,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            kind: "tv".into(),
            allow_http: false,
            allow_http_logos: false,
            cookies: false,
            encoding: "utf-8".into(),
            format: "extended".into(),
            group_profile: "default".into(),
            content: "auto".into(),
            vod_detection: "prudent".into(),
            group_content: BTreeMap::new(),
            refresh_minutes: 0,
            access: BTreeMap::new(),
            guide: GuideSettings::default(),
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        if !["tv", "radio"].contains(&self.kind.as_str())
            || !["utf-8", "windows-1252", "latin-1"].contains(&self.encoding.as_str())
            || !["extended", "simple"].contains(&self.format.as_str())
            || !["default", "kodi", "ott"].contains(&self.group_profile.as_str())
            || !["auto", "live", "vod"].contains(&self.content.as_str())
            || !["prudent", "extended"].contains(&self.vod_detection.as_str())
            || self.group_content.len() > 100
            || self.group_content.iter().any(|(name, kind)| {
                name.trim().is_empty()
                    || name != name.trim()
                    || name.chars().count() > 100
                    || name.chars().any(char::is_control)
                    || !["live", "vod"].contains(&kind.as_str())
            })
            || (self.refresh_minutes != 0 && !(15..=10_080).contains(&self.refresh_minutes))
            || self.guide.urls.len() > 16
            || self.guide.mappings.len() > 100_000
            || !(1..=7).contains(&self.guide.history_days)
            || !(1..=48).contains(&self.guide.future_hours)
            || !(-1440..=1440).contains(&self.guide.shift_minutes)
            || self.guide.before_seconds > 300
            || self.guide.after_seconds > 300
            || self.guide.archive_days.is_some_and(|d| d > 365)
            || self.guide.language.len() > 20
        {
            bail!("Impostazioni della lista non valide.");
        }
        let mut groups = std::collections::BTreeSet::new();
        if self
            .group_content
            .keys()
            .any(|g| !groups.insert(g.to_lowercase()))
        {
            bail!("Esiste già una regola per questo gruppo.");
        }
        for zone in [&self.guide.timezone, &self.guide.archive_timezone] {
            if zone.parse::<chrono_tz::Tz>().is_err() {
                bail!("Fuso orario non valido.");
            }
        }
        for access in self.access.values() {
            network::validate_headers(&access.headers)?;
            if access.origins.len() > 16 {
                bail!("Sono ammessi fino a 16 server per funzione.");
            }
            for origin in &access.origins {
                network::origin(origin)?;
            }
        }
        for url in &self.guide.urls {
            network::validate_url(url, self.guide.allow_http)?;
        }
        Ok(())
    }
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Metadata {
    pub duration: f64,
    pub tvg_id: String,
    pub tvg_name: String,
    pub guide_urls: Vec<String>,
    pub shift_hours: f64,
    pub attributes: BTreeMap<String, String>,
    pub properties: BTreeMap<String, String>,
    pub catchup_id: String,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub number: Option<u32>,
    pub kind: crate::model::MediaKind,
    pub content: String,
    pub groups: Vec<String>,
    pub logo: String,
    pub url: String,
    pub headers: Headers,
    pub metadata: Metadata,
    pub position: usize,
}
impl Entry {
    pub fn display(&self, source: &str, settings: &Settings) -> DisplayChannel {
        let content = if settings.content == "auto" {
            &self.content
        } else {
            &settings.content
        };
        DisplayChannel {
            channel: CatalogChannel {
                kind: self.kind,
                id: self.id.clone(),
                name: self.name.clone(),
                number: self.number.unwrap_or(9999),
                provider: "personal".into(),
                epg_id: String::new(),
                key: String::new(),
                headers: BTreeMap::new(),
                logo: String::new(),
                group: self.groups.join(" · "),
                color: "#7e91ac".into(),
            },
            source_id: source.into(),
            content: content.into(),
            position: self.position,
            now: None,
            next: None,
            guide_origin: String::new(),
            artwork_revision: String::new(),
            hints: metadata::Hints::from(self),
        }
    }
}
#[derive(Clone, Serialize)]
pub struct DisplayChannel {
    #[serde(flatten)]
    pub channel: CatalogChannel,
    pub source_id: String,
    pub content: String,
    pub position: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub now: Option<crate::guide::Programme>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<crate::guide::Programme>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub guide_origin: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub artwork_revision: String,
    #[serde(skip)]
    hints: metadata::Hints,
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct ImportReport {
    pub scanned: usize,
    pub count: usize,
    pub excluded: usize,
    pub duplicates: usize,
    pub vod: usize,
    pub radio: usize,
    pub bytes: u64,
    pub sample: Vec<String>,
    pub diagnostics: BTreeMap<String, usize>,
    pub guide_urls: Vec<String>,
    pub refresh_minutes: u32,
}
impl ImportReport {
    pub fn exclude(&mut self, message: &str) {
        self.excluded += 1;
        if self.diagnostics.contains_key(message) || self.diagnostics.len() < 20 {
            *self.diagnostics.entry(message.into()).or_default() += 1;
        }
    }
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct GuideStatus {
    pub updated_at: u64,
    pub matched: usize,
    pub programmes: usize,
    pub loading: bool,
    pub error: String,
    pub next_retry_at: u64,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Source {
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub(super) android_descriptor: serde_json::Value,
    #[serde(default)]
    pub snapshot: String,
    pub id: String,
    pub name: String,
    pub url: Option<String>,
    pub enabled: bool,
    pub revision: u64,
    pub updated_at: u64,
    pub checked_at: u64,
    pub retry_at: u64,
    pub etag: String,
    pub modified: String,
    pub settings: Settings,
    pub report: ImportReport,
    pub guide: GuideStatus,
    pub error: String,
}
impl Source {
    pub fn public(&self) -> serde_json::Value {
        let mut report = self.report.clone();
        report.guide_urls.clear();
        serde_json::json!({"id":self.id,"name":self.name,"enabled":self.enabled,
            "revision":self.revision,"updated_at":self.updated_at,"checked_at":self.checked_at,
            "retry_at":self.retry_at,"report":report,"guide":self.guide,"error":self.error,
            "file":self.url.is_none(),"has_vod":self.enabled && self.report.count>0 &&
                (self.settings.content=="vod" || self.settings.content=="auto" && self.report.vod>0)})
    }
}

pub struct PersonalPlayback {
    pub stream: ResolvedStream,
    pub policy: Arc<network::SessionPolicy>,
    pub channel: DisplayChannel,
    pub mode: String,
    pub title: String,
    pub position: f64,
    pub duration: f64,
    pub base_offset: f64,
    pub event: String,
    pub seekable: bool,
    pub watch_revision: u64,
}

#[cfg(test)]
mod tests;
