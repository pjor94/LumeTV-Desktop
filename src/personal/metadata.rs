// SPDX-License-Identifier: GPL-3.0-or-later
//! Public metadata supplements missing list declarations without changing playback identity.
use super::*;
use crate::public_guide::Identity;
use std::sync::Weak;

#[derive(Clone, Default)]
pub(super) struct Hints {
    pub xml_id: String,
    pub tvg_name: String,
    pub logo_provided: bool,
    pub guide_provided: bool,
}
impl From<&Entry> for Hints {
    fn from(entry: &Entry) -> Self {
        let nonempty = |key: &str| {
            entry
                .metadata
                .attributes
                .get(key)
                .is_some_and(|v| !v.trim().is_empty())
        };
        Self {
            xml_id: entry.metadata.tvg_id.clone(),
            tvg_name: entry.metadata.tvg_name.clone(),
            logo_provided: !entry.logo.trim().is_empty() || nonempty("tvg-logo"),
            guide_provided: !entry.metadata.guide_urls.is_empty()
                || nonempty("url-tvg")
                || nonempty("x-tvg-url"),
        }
    }
}
impl Hints {
    pub fn identity(&self, name: &str) -> Identity {
        Identity {
            xml_id: self.xml_id.clone(),
            names: vec![self.tvg_name.clone(), name.into()],
        }
    }
}
impl Source {
    pub(super) fn has_guide(&self) -> bool {
        !self.settings.guide.urls.is_empty() || !self.report.guide_urls.is_empty()
    }
}
impl Library {
    pub(crate) fn attach_public_guide(&self, guide: Weak<crate::guide::GuideService>) {
        *self.public_guide.write().expect("public guide link") = guide;
    }
    pub(super) fn shared_guide(&self) -> Option<Arc<crate::guide::GuideService>> {
        self.public_guide
            .read()
            .expect("public guide link")
            .upgrade()
    }
    pub(super) fn public_metadata_revision(&self) -> String {
        self.shared_guide()
            .map(|g| g.metadata_revision())
            .unwrap_or_default()
    }
    pub(super) fn presented_guide(&self, source: &Source) -> serde_json::Value {
        if !source.has_guide() {
            if let Some(shared) = self.shared_guide() {
                return shared.public_status();
            }
        }
        let mut value = serde_json::to_value(&source.guide).unwrap_or_default();
        value["origin"] = serde_json::json!(if source.has_guide() {
            "personal"
        } else {
            "none"
        });
        value
    }
    pub(super) fn use_public_programmes(&self, source: &Source, id: &str, hints: &Hints) -> bool {
        !source.has_guide()
            && !hints.guide_provided
            && !source
                .settings
                .guide
                .mappings
                .get(id)
                .is_some_and(String::is_empty)
    }
    pub(super) fn decorate_channels(&self, source: &Source, channels: &mut [DisplayChannel]) {
        let owned = if source.has_guide() && source.settings.guide.enabled {
            self.guide_path(&source.id)
                .ok()
                .filter(|p| p.exists())
                .and_then(|p| super::store::open(&p).ok())
        } else {
            None
        };
        let shared = self.shared_guide();
        let revision = shared
            .as_ref()
            .map(|g| g.metadata_revision())
            .unwrap_or_default();
        let mut indexes = Vec::new();
        let mut hints = Vec::new();
        for (i, channel) in channels.iter_mut().enumerate() {
            channel.artwork_revision = source.revision.to_string();
            if channel.content != "live" || channel.channel.kind != crate::model::MediaKind::Tv {
                continue;
            }
            if let Some(db) = &owned {
                Self::decorate(db, channel);
                channel.guide_origin = "personal".into();
            }
            if shared.is_some()
                && (!channel.hints.logo_provided
                    || self.use_public_programmes(source, &channel.channel.id, &channel.hints))
            {
                indexes.push(i);
                hints.push(channel.hints.identity(&channel.channel.name));
                if !channel.hints.logo_provided {
                    channel.artwork_revision = format!("{}:{revision}", source.revision);
                }
            }
        }
        if let Some(shared) = shared {
            for (i, metadata) in indexes.into_iter().zip(shared.lookup(&hints)) {
                let channel = &mut channels[i];
                if self.use_public_programmes(source, &channel.channel.id, &channel.hints) {
                    channel.guide_origin = "lume".into();
                    if let Some(metadata) = metadata {
                        if metadata.has_programmes {
                            channel.guide_origin = metadata.origin.into();
                        }
                        channel.now = metadata.now;
                        channel.next = metadata.next;
                    }
                }
            }
        }
    }
    pub(crate) fn public_logo_candidates(&self, id: &str) -> Result<Vec<String>> {
        let (_, entry) = self.entry(id)?;
        let hints = Hints::from(&entry);
        if hints.logo_provided
            || entry.kind != crate::model::MediaKind::Tv
            || entry.content != "live"
        {
            return Ok(vec![]);
        }
        Ok(self
            .shared_guide()
            .map(|g| g.public_logo_candidates(&hints.identity(&entry.name)))
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::personal::tests::Fixture;
    use std::io::Cursor;

    fn shared(f: &Fixture) -> Arc<crate::guide::GuideService> {
        let guide = crate::guide::GuideService::new(&f.root.join("public")).unwrap();
        let now = chrono::Utc::now();
        let start = (now - chrono::Duration::minutes(20)).format("%Y%m%d%H%M%S +0000");
        let end = (now + chrono::Duration::minutes(40)).format("%Y%m%d%H%M%S +0000");
        let xml = format!(
            r#"<tv><channel id="Extra.it"><display-name>Canale Esteso</display-name><icon src="https://images.example.org/extra.png"/></channel><programme channel="Extra.it" start="{start}" stop="{end}"><title lang="it">Programma dal feed completo</title><desc>Descrizione pubblica</desc></programme></tv>"#
        );
        guide
            .import_xml(Cursor::new(xml), crate::report::now_ms())
            .unwrap();
        let extra = format!(
            r#"<tv><channel id="Sky.Uno.it"><display-name>Sky Uno</display-name><icon src="https://guidatv.sky.it/logo/uno.png"/></channel><channel id="Extra.it"><display-name>Canale Esteso</display-name></channel><programme channel="Sky.Uno.it" start="{start}" stop="{end}"><title>Programma da EPGShare</title></programme><programme channel="Extra.it" start="{start}" stop="{end}"><title>Programma supplementare da ignorare</title></programme></tv>"#
        );
        guide
            .import_epgshare_xml(Cursor::new(extra), crate::report::now_ms())
            .unwrap();
        f.library.attach_public_guide(Arc::downgrade(&guide));
        guide
    }
    #[test]
    fn missing_metadata_uses_the_full_feed_on_every_surface_without_changing_playback() {
        let f = Fixture::new();
        let source=f.import("#EXTM3U\n#EXTINF:-1 tvg-id=\"Extra.it\",Canale Esteso HD\nhttps://private.example/live.mpd?token=private-fixture\n",None);
        let _guide = shared(&f);
        let page = f.rows(&source);
        let id = page["rows"][0]["id"].as_str().unwrap();
        let check = |row: &serde_json::Value| {
            assert_eq!(row["now"]["title"], "Programma dal feed completo");
            assert_eq!(row["guide_origin"], "lume");
            assert!(!row.to_string().contains("private-fixture"));
            assert!(!row.to_string().contains("images.example.org"));
        };
        check(&page["rows"][0]);
        check(&serde_json::to_value(f.library.channel(id).unwrap()).unwrap());
        check(&f.library.home(&source, &[]).unwrap()["featured"]);
        check(&serde_json::to_value(f.library.next(id, 1, &[], false).unwrap().unwrap()).unwrap());
        let playback = f.library.playback(id, "live", "", None).unwrap();
        check(&serde_json::to_value(playback.channel).unwrap());
        assert_eq!(
            playback.stream.url,
            "https://private.example/live.mpd?token=private-fixture"
        );
        assert_eq!(
            f.library.guide_window(id, 0, u64::MAX, "", 0).unwrap()["total"],
            0
        );
        let window = f
            .library
            .guide_window(
                id,
                crate::report::now_ms() - 3_600_000,
                crate::report::now_ms() + 3_600_000,
                "",
                0,
            )
            .unwrap();
        assert_eq!(window["rows"][0]["title"], "Programma dal feed completo");
        assert_eq!(window["rows"][0]["archive"], false);
        assert_eq!(
            f.library.public_logo_candidates(id).unwrap(),
            vec!["https://images.example.org/extra.png"]
        );
        assert_eq!(
            f.library.snapshot()["sources"][0]["guide"]["origin"],
            "lume"
        );
    }
    #[test]
    fn declared_logo_and_personal_guide_stay_authoritative_even_when_unavailable() {
        let f = Fixture::new();
        let source=f.import("#EXTM3U url-tvg=\"https://private.example/guide.xml\"\n#EXTINF:-1 tvg-id=\"Extra.it\" tvg-logo=\"https://private.example/logo.png\",Canale Esteso\nhttps://private.example/live.mpd\n#EXTINF:-1 tvg-id=\"Extra.it\",Canale senza logo\nhttps://private.example/second.mpd\n",None);
        let _guide = shared(&f);
        let page = f.rows(&source);
        let id = page["rows"][0]["id"].as_str().unwrap();
        assert!(page["rows"][0].get("now").is_none());
        assert!(f.library.public_logo_candidates(id).unwrap().is_empty());
        assert_eq!(
            f.library.guide_window(id, 0, u64::MAX, "", 0).unwrap()["total"],
            0
        );
        {
            let mut sources = f.library.sources.lock().unwrap();
            sources.get_mut(&source).unwrap().settings.guide.enabled = true;
        }
        let db = super::super::store::open(&f.library.guide_path(&source).unwrap()).unwrap();
        db.execute_batch("CREATE TABLE programmes(channel TEXT,title TEXT,start INTEGER,end INTEGER,description TEXT,catchup_id TEXT);").unwrap();
        let at = crate::report::now_ms();
        db.execute(
            "INSERT INTO programmes VALUES(?,?,?,?,?,?)",
            rusqlite::params![
                id,
                "Programma personale",
                at - 60_000,
                at + 60_000,
                "Trama personale",
                ""
            ],
        )
        .unwrap();
        assert_eq!(
            f.library.channel(id).unwrap().now.unwrap().title,
            "Programma personale"
        );
        assert_eq!(
            f.library.snapshot()["sources"][0]["guide"]["origin"],
            "personal"
        );
        // A personal EPG does not prevent an independently missing logo from being filled.
        let missing_logo = page["rows"][1]["id"].as_str().unwrap();
        assert_eq!(
            f.library.public_logo_candidates(missing_logo).unwrap(),
            vec!["https://images.example.org/extra.png"]
        );
        assert!(f.library.channel(missing_logo).unwrap().now.is_none());
    }
    #[test]
    fn epgshare_completes_missing_live_metadata_but_never_declared_personal_fields() {
        let f = Fixture::new();
        let source = f.import("#EXTM3U\n#EXTINF:-1 tvg-id=\"Sky.Uno.it\",Sky Uno\nhttps://private.example/sky.mpd?token=private-fixture\n#EXTINF:-1 tvg-id=\"Sky.Uno.it\" tvg-logo=\"https://private.example/logo.png\",Sky Uno con logo\nhttps://private.example/sky2.mpd\n",None);
        let _guide = shared(&f);
        let rows = f.rows(&source);
        let id = rows["rows"][0]["id"].as_str().unwrap();
        assert_eq!(rows["rows"][0]["guide_origin"], "epgshare");
        assert_eq!(rows["rows"][0]["now"]["title"], "Programma da EPGShare");
        assert_eq!(
            f.library.public_logo_candidates(id).unwrap(),
            vec!["https://guidatv.sky.it/logo/uno.png"]
        );
        let declared = rows["rows"][1]["id"].as_str().unwrap();
        assert!(f
            .library
            .public_logo_candidates(declared)
            .unwrap()
            .is_empty());
        let window = f
            .library
            .guide_window(
                id,
                crate::report::now_ms() - 3_600_000,
                crate::report::now_ms() + 3_600_000,
                "",
                0,
            )
            .unwrap();
        assert_eq!(window["origin"], "epgshare");
        assert_eq!(window["rows"][0]["archive"], false);
        assert!(window["rows"][0]["event"]
            .as_str()
            .unwrap()
            .starts_with("epgshare:"));
        let playback = f.library.playback(id, "live", "", None).unwrap();
        assert_eq!(
            playback.stream.url,
            "https://private.example/sky.mpd?token=private-fixture"
        );
        assert_eq!(playback.channel.guide_origin, "epgshare");
        {
            let mut sources = f.library.sources.lock().unwrap();
            sources.get_mut(&source).unwrap().settings.guide.urls =
                vec!["https://private.example/guide.xml".into()];
        }
        assert!(f.library.channel(id).unwrap().now.is_none());
        assert_eq!(
            f.library.guide_window(id, 0, u64::MAX, "", 0).unwrap()["total"],
            0
        );
        // The declared EPG blocks both public schedules, but the missing logo stays independent.
        assert_eq!(
            f.library.public_logo_candidates(id).unwrap(),
            vec!["https://guidatv.sky.it/logo/uno.png"]
        );
    }

    #[test]
    fn logo_and_guide_are_independent_and_manual_exclusions_and_vod_are_respected() {
        let f = Fixture::new();
        let source=f.import("#EXTM3U\n#EXTINF:-1 tvg-id=\"Extra.it\" tvg-logo=\"https://private.example/logo.png\",Canale Esteso\nhttps://private.example/live.mpd\n#EXTINF:300 type=\"movie\" tvg-id=\"Extra.it\",Un film\nhttps://private.example/movie.mp4\n",None);
        let _guide = shared(&f);
        let page = f.rows(&source);
        let id = page["rows"][0]["id"].as_str().unwrap();
        assert_eq!(
            page["rows"][0]["now"]["title"],
            "Programma dal feed completo"
        );
        assert!(f.library.public_logo_candidates(id).unwrap().is_empty());
        let vod = page["rows"][1]["id"].as_str().unwrap();
        assert!(page["rows"][1].get("now").is_none());
        assert!(f.library.public_logo_candidates(vod).unwrap().is_empty());
        assert_eq!(
            f.library.guide_window(vod, 0, u64::MAX, "", 0).unwrap()["total"],
            0
        );
        f.library
            .sources
            .lock()
            .unwrap()
            .get_mut(&source)
            .unwrap()
            .settings
            .guide
            .mappings
            .insert(id.into(), String::new());
        assert!(f.library.channel(id).unwrap().now.is_none());
    }
}
