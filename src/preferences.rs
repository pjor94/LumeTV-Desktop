// SPDX-License-Identifier: GPL-3.0-or-later
use crate::catalog::{valid_id, CatalogService};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Reminder {
    pub channel_id: String,
    pub start: u64,
    pub title: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Preferences {
    pub favorites: Vec<String>,
    pub recent: Vec<String>,
    pub reminders: Vec<Reminder>,
    pub theme: String,
    pub volume: f64,
    pub muted: bool,
    pub quality: String,
    pub zap_favorites_only: bool,
    pub audio_language: String,
    pub subtitle_mode: String,
    pub subtitle_language: String,
    pub channel_layout: Value,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            favorites: vec![],
            recent: vec![],
            reminders: vec![],
            theme: "dark".into(),
            volume: 0.7,
            muted: false,
            quality: "auto".into(),
            zap_favorites_only: false,
            audio_language: String::new(),
            subtitle_mode: "system".into(),
            subtitle_language: String::new(),
            channel_layout: serde_json::json!({"version":1,"order":[],"hidden":[],"owners":{}}),
        }
    }
}

pub(crate) fn valid_language(value: &str) -> bool {
    static LANGUAGE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^[a-zA-Z]{2,3}(?:-[a-zA-Z0-9]{2,8})*$").unwrap()
    });
    value.is_empty() || value.len() <= 35 && LANGUAGE.is_match(value)
}

pub struct PreferenceStore {
    catalog: Arc<CatalogService>,
    path: PathBuf,
    value: Mutex<Preferences>,
}

impl PreferenceStore {
    pub fn open(directory: &Path) -> Result<Self> {
        Self::with_catalog(directory, CatalogService::new(directory)?)
    }
    pub fn with_catalog(directory: &Path, catalog: Arc<CatalogService>) -> Result<Self> {
        fs::create_dir_all(directory).context("Impossibile preparare i dati di Lume TV")?;
        let path = directory.join("preferences.json");
        let mut value = if path.is_file() {
            let bytes = fs::read(&path).context("Impossibile leggere le preferenze")?;
            if bytes.len() > 8 * 1024 * 1024 {
                bail!("File preferenze troppo grande");
            }
            serde_json::from_slice(&bytes).context("Preferenze non valide")?
        } else {
            Preferences::default()
        };
        normalize(&mut value);
        catalog.personal.set_layout(&value.channel_layout);
        Ok(Self {
            catalog,
            path,
            value: Mutex::new(value),
        })
    }

    pub fn get(&self) -> Preferences {
        self.value.lock().expect("preferences lock").clone()
    }

    pub(crate) fn replace_backup(&self, mut value: Preferences) -> Result<Preferences> {
        let mut guard = self
            .value
            .lock()
            .map_err(|_| anyhow::anyhow!("Preferenze occupate"))?;
        normalize(&mut value);
        let temp = self
            .path
            .with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let bytes = serde_json::to_vec_pretty(&value)?;
        if bytes.len() > 8 * 1024 * 1024 {
            bail!("Le preferenze superano il limite di spazio.");
        }
        fs::write(&temp, bytes)?;
        if let Err(error) = fs::rename(&temp, &self.path) {
            let _ = fs::remove_file(&temp);
            return Err(error.into());
        }
        self.catalog.personal.set_layout(&value.channel_layout);
        *guard = value.clone();
        Ok(value)
    }
    pub fn update(&self, patch: &Value) -> Result<Preferences> {
        let mut guard = self
            .value
            .lock()
            .map_err(|_| anyhow::anyhow!("Preferenze occupate"))?;
        let mut next = guard.clone();
        for key in ["audio_language", "subtitle_language"] {
            if let Some(language) = patch[key].as_str() {
                if !valid_language(language) {
                    bail!("Lingua non valida.");
                }
                if key == "audio_language" {
                    next.audio_language = language.into();
                } else {
                    next.subtitle_language = language.into();
                }
            }
        }
        if let Some(mode) = patch["subtitle_mode"].as_str() {
            if !["system", "off", "language"].contains(&mode) {
                bail!("Preferenza sottotitoli non valida.");
            }
            next.subtitle_mode = mode.into();
        }
        if let Some(id) = patch["hide_channel"].as_str() {
            let channel = self
                .catalog
                .channel(id)
                .ok_or_else(|| anyhow::anyhow!("Canale non disponibile."))?;
            let source = if id.starts_with("personal_") {
                self.catalog
                    .personal
                    .channel(id)
                    .map(|c| c.source_id)
                    .unwrap_or_default()
            } else if channel.kind == crate::model::MediaKind::Radio {
                "builtin_radio".into()
            } else {
                "builtin_tv".into()
            };
            let rows = next.channel_layout["hidden"]
                .as_array_mut()
                .ok_or_else(|| anyhow::anyhow!("Personalizzazioni non valide."))?;
            if !rows.iter().any(|r| r["id"] == id) {
                rows.push(serde_json::json!({"id":id,"source":source,"name":channel.name.chars().take(160).collect::<String>(),"kind":if channel.kind==crate::model::MediaKind::Radio{"RADIO"}else{"TV"}}));
            }
        }
        if let Some(id) = patch["restore_hidden"].as_str() {
            if let Some(rows) = next.channel_layout["hidden"].as_array_mut() {
                rows.retain(|r| r["id"] != id);
            }
        }
        if let Some(id) = patch["channel_first"].as_str() {
            if self.catalog.channel(id).is_none() {
                bail!("Canale non disponibile.");
            }
            let rows = next.channel_layout["order"]
                .as_array_mut()
                .ok_or_else(|| anyhow::anyhow!("Ordine dei canali non valido."))?;
            rows.retain(|v| v != id);
            rows.insert(0, serde_json::json!(id));
        }
        if patch["reset_channel_order"] == true {
            next.channel_layout["order"] = serde_json::json!([]);
        }
        if let Some(id) = patch["toggle_favorite"].as_str() {
            if self.catalog.channel(id).is_none() {
                bail!("Canale sconosciuto");
            }
            if next.favorites.iter().any(|v| v == id) {
                next.favorites.retain(|v| v != id);
            } else {
                next.favorites.push(id.to_owned());
            }
        }
        if let Some(id) = patch["played"].as_str() {
            if self.catalog.channel(id).is_none() {
                bail!("Canale sconosciuto");
            }
            next.recent.retain(|v| v != id);
            next.recent.insert(0, id.to_owned());
            next.recent.truncate(40);
        }
        if patch["clear_recent"] == true {
            next.recent.clear();
        }
        if let Some(theme) = patch["theme"].as_str() {
            if !["dark", "light", "system"].contains(&theme) {
                bail!("Aspetto non valido");
            }
            next.theme = theme.to_owned();
        }
        if let Some(quality) = patch["quality"].as_str() {
            if !["auto", "1080", "720", "480"].contains(&quality) {
                bail!("Qualità non valida");
            }
            next.quality = quality.to_owned();
        }
        if let Some(volume) = patch["volume"].as_f64() {
            if !(0.0..=1.0).contains(&volume) {
                bail!("Volume non valido");
            }
            next.volume = volume;
        }
        if let Some(muted) = patch["muted"].as_bool() {
            next.muted = muted;
        }
        if let Some(only) = patch["zap_favorites_only"].as_bool() {
            next.zap_favorites_only = only;
        }
        if let Some(reminder) = patch.get("toggle_reminder") {
            let reminder: Reminder =
                serde_json::from_value(reminder.clone()).context("Promemoria non valido")?;
            if self.catalog.channel(&reminder.channel_id).is_none()
                || reminder.title.chars().count() > 300
                || reminder.title.trim().is_empty()
            {
                bail!("Promemoria non valido");
            }
            let existed = next
                .reminders
                .iter()
                .any(|r| r.channel_id == reminder.channel_id && r.start == reminder.start);
            next.reminders
                .retain(|r| !(r.channel_id == reminder.channel_id && r.start == reminder.start));
            if !existed {
                next.reminders.push(reminder);
            }
        }
        normalize(&mut next);
        let temporary = self
            .path
            .with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let bytes = serde_json::to_vec_pretty(&next)?;
        if bytes.len() > 8 * 1024 * 1024 {
            bail!("Le preferenze superano il limite di spazio.");
        }
        fs::write(&temporary, bytes).context("Impossibile salvare le preferenze")?;
        if fs::rename(&temporary, &self.path).is_err() {
            let _ = fs::remove_file(&temporary);
            bail!("Impossibile aggiornare le preferenze");
        }
        self.catalog.personal.set_layout(&next.channel_layout);
        *guard = next.clone();
        Ok(next)
    }
}

fn normalize(value: &mut Preferences) {
    if value.channel_layout["order"].as_array().is_none()
        || value.channel_layout["hidden"].as_array().is_none()
    {
        value.channel_layout = serde_json::json!({"version":1,"order":[],"hidden":[],"owners":{}});
    }
    for language in [&mut value.audio_language, &mut value.subtitle_language] {
        if !valid_language(language) {
            language.clear();
        }
    }
    if !["system", "off", "language"].contains(&value.subtitle_mode.as_str()) {
        value.subtitle_mode = "system".into();
    }

    let mut seen = std::collections::HashSet::new();
    value
        .favorites
        .retain(|id| valid_id(id) && seen.insert(id.clone()));
    seen.clear();
    value
        .recent
        .retain(|id| valid_id(id) && seen.insert(id.clone()));
    value.recent.truncate(40);
    value
        .reminders
        .retain(|r| valid_id(&r.channel_id) && r.title.chars().count() <= 300);
    value.reminders.truncate(64);
    if !["dark", "light", "system"].contains(&value.theme.as_str()) {
        value.theme = "dark".into();
    }
    if !["auto", "1080", "720", "480"].contains(&value.quality.as_str()) {
        value.quality = "auto".into();
    }
    if !value.volume.is_finite() || !(0.0..=1.0).contains(&value.volume) {
        value.volume = 0.7;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn preferences_accept_new_cached_channels_and_keep_favorites_when_a_feed_temporarily_loses_one()
    {
        let dir = std::env::temp_dir().join(format!("lume-new-favorite-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let playlist=format!("{}\n#EXTINF:-1 tvg-id=\"newfixture\" group-title=\"News\",New fixture\nhttps://media.example/live.m3u8\n",crate::catalog::BUNDLED_PLAYLIST);
        fs::write(
            dir.join("catalog.json"),
            serde_json::to_vec(&json!({"playlist":playlist,"last_success":1})).unwrap(),
        )
        .unwrap();
        let catalogue = CatalogService::new(&dir).unwrap();
        let id = catalogue
            .snapshot()
            .channels
            .iter()
            .find(|c| c.epg_id == "newfixture")
            .unwrap()
            .id
            .clone();
        let store = PreferenceStore::with_catalog(&dir, catalogue).unwrap();
        store.update(&json!({"toggle_favorite":id})).unwrap();
        fs::remove_file(dir.join("catalog.json")).unwrap();
        let restored = PreferenceStore::open(&dir).unwrap();
        assert!(restored.get().favorites.contains(&id));
        assert!(restored
            .update(&json!({"toggle_favorite":"not-a-channel"}))
            .is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn favorites_recent_and_settings_survive_a_restart_without_duplicate_history() {
        let dir = std::env::temp_dir().join(format!("lume-preferences-{}", uuid::Uuid::new_v4()));
        let store = PreferenceStore::open(&dir).unwrap();
        store
            .update(
                &json!({"toggle_favorite":"rai1", "played":"rai1", "volume":0.4, "theme":"system"}),
            )
            .unwrap();
        store.update(&json!({"played":"canale5"})).unwrap();
        store.update(&json!({"played":"rai1"})).unwrap();
        let restored = PreferenceStore::open(&dir).unwrap().get();
        assert_eq!(restored.favorites, ["rai1"]);
        assert_eq!(restored.recent, ["rai1", "canale5"]);
        assert_eq!(restored.volume, 0.4);
        assert_eq!(restored.theme, "system");
        assert!(store
            .update(&json!({"played":"https://untrusted.test"}))
            .is_err());
        assert!(store.update(&json!({"volume":2})).is_err());
        assert_eq!(store.get().volume, 0.4);
        fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod android_preferences_tests {
    use super::*;
    #[test]
    fn language_tags_match_android_including_numeric_regions() {
        for language in ["", "it", "eng", "es-419", "zh-Hant-TW"] {
            assert!(valid_language(language));
        }
        for language in [
            "i",
            "italian",
            "it_ITALY",
            "it--IT",
            "it-a",
            "https://example",
        ] {
            assert!(!valid_language(language));
        }
    }
}
