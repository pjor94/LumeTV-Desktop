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
        }
    }
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
            if bytes.len() > 64 * 1024 {
                bail!("File preferenze troppo grande");
            }
            serde_json::from_slice(&bytes).context("Preferenze non valide")?
        } else {
            Preferences::default()
        };
        normalize(&mut value);
        Ok(Self {
            catalog,
            path,
            value: Mutex::new(value),
        })
    }

    pub fn get(&self) -> Preferences {
        self.value.lock().expect("preferences lock").clone()
    }

    pub fn update(&self, patch: &Value) -> Result<Preferences> {
        let mut guard = self
            .value
            .lock()
            .map_err(|_| anyhow::anyhow!("Preferenze occupate"))?;
        let mut next = guard.clone();
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
        fs::write(&temporary, bytes).context("Impossibile salvare le preferenze")?;
        if fs::rename(&temporary, &self.path).is_err() {
            let _ = fs::remove_file(&temporary);
            bail!("Impossibile aggiornare le preferenze");
        }
        *guard = next.clone();
        Ok(next)
    }
}

fn normalize(value: &mut Preferences) {
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
