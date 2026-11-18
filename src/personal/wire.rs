// SPDX-License-Identifier: GPL-3.0-or-later
//! Android's versioned playlist JSONL representation, shared by backup and companion.
use super::*;
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(crate) fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
pub(crate) fn channel_id(id: &str) -> String {
    let suffix = id.strip_prefix("personal_").unwrap_or(id);
    if suffix.len() == 24 && suffix.bytes().all(|b| b.is_ascii_hexdigit()) {
        suffix.to_owned()
    } else {
        hash(id)[..24].to_owned()
    }
}
pub(crate) fn builtin_id(c: &CatalogChannel) -> String {
    if c.id.len() == 24 && c.id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return c.id.clone();
    }
    hash(&if c.provider == "direct" {
        format!("{}|{}|{}", c.epg_id, c.name, c.group)
    } else {
        format!("{}|{}", c.provider, c.key)
    })[..24]
        .to_owned()
}
fn android_headers(headers: &Headers) -> Headers {
    headers
        .iter()
        .map(|(key, value)| {
            let key = key.trim_start_matches('!').to_ascii_lowercase();
            let key = match key.as_str() {
                "referrer" => "referer",
                "cookies" => "cookie",
                _ => &key,
            };
            (
                key.split('-')
                    .map(|s| {
                        let mut c = s.chars();
                        c.next()
                            .map(|first| first.to_ascii_uppercase().to_string() + c.as_str())
                            .unwrap_or_default()
                    })
                    .collect::<Vec<_>>()
                    .join("-"),
                value.clone(),
            )
        })
        .collect()
}
pub(crate) fn settings(s: &Settings) -> Value {
    let access = s
        .access
        .iter()
        .map(|(role, a)| {
            (
                if *role == Resource::Guide {
                    "EPG".into()
                } else {
                    serde_json::to_value(role)
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .to_uppercase()
                },
                json!({"headers":android_headers(&a.headers),"origins":a.origins.iter().filter_map(|v|reqwest::Url::parse(v).ok()).map(|u|{let origin=u.origin().ascii_serialization();if u.port().is_none(){format!("{origin}:{}",u.port_or_known_default().unwrap_or(443))}else{origin}}).collect::<std::collections::BTreeSet<_>>()}),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let g = &s.guide;
    json!({"format":if s.format=="simple" {"PLAIN"} else {"EXTENDED"},"encoding":match s.encoding.as_str(){"windows-1252"=>"WINDOWS_1252","latin-1"=>"LATIN1",_=>"UTF8"},"groups":match s.group_profile.as_str(){"kodi"=>"KODI","ott"=>"OTT",_=>"CURRENT"},"vodDetection":s.vod_detection.to_uppercase(),"groupContent":s.group_content.iter().map(|(k,v)|(k.clone(),Value::String(v.to_uppercase()))).collect::<serde_json::Map<_,_>>(),"cookies":s.cookies,"artworkHttp":s.allow_http_logos,"refreshMinutes":if s.refresh_minutes==0 {Value::Null} else {json!(s.refresh_minutes)},"contentOverride":if s.content=="auto" {Value::Null} else {json!(s.content.to_uppercase())},"access":access,"guide":{"enabled":g.enabled,"http":g.allow_http,"urls":g.urls,"history":g.history_days,"future":g.future_hours,"language":g.language,"xmlZone":g.timezone,"shift":g.shift_minutes as f64/60.0,"archiveZone":g.archive_timezone,"archiveDays":g.archive_days,"ongoing":g.ongoing,"before":g.before_seconds,"after":g.after_seconds}})
}
pub(crate) fn read_settings(h: &Value) -> Result<Settings> {
    let v = &h["settings"];
    let g = &v["guide"];
    let mut s = Settings {
        kind: if h["kind"] == "RADIO" { "radio" } else { "tv" }.into(),
        allow_http: h["allowHttp"].as_bool().unwrap_or(false),
        ..Default::default()
    };
    s.format = if v["format"] == "PLAIN" {
        "simple"
    } else {
        "extended"
    }
    .into();
    s.encoding = match v["encoding"].as_str().unwrap_or("UTF8") {
        "UTF8" => "utf-8",
        "WINDOWS_1252" => "windows-1252",
        "LATIN1" => "latin-1",
        _ => bail!("Codifica del backup non supportata."),
    }
    .into();
    s.group_profile = match v["groups"].as_str().unwrap_or("CURRENT") {
        "CURRENT" => "default",
        "KODI" => "kodi",
        "OTT" => "ott",
        _ => bail!("Gruppi del backup non validi."),
    }
    .into();
    s.vod_detection = v["vodDetection"]
        .as_str()
        .unwrap_or("PRUDENT")
        .to_lowercase();
    if let Some(rules) = v["groupContent"].as_object() {
        for (k, v) in rules {
            s.group_content.insert(
                k.clone(),
                v.as_str()
                    .ok_or_else(|| anyhow!("Regola del gruppo non valida."))?
                    .to_lowercase(),
            );
        }
    }
    s.content = v["contentOverride"]
        .as_str()
        .unwrap_or("auto")
        .to_lowercase();
    s.cookies = v["cookies"].as_bool().unwrap_or(false);
    s.allow_http_logos = v["artworkHttp"].as_bool().unwrap_or(false);
    s.refresh_minutes = v["refreshMinutes"].as_u64().unwrap_or(0).try_into()?;
    if let Some(access) = v["access"].as_object() {
        for (role, rule) in access {
            let role = if role == "EPG" {
                "guide".into()
            } else {
                role.to_lowercase()
            };
            s.access.insert(
                serde_json::from_value(json!(role))?,
                serde_json::from_value(rule.clone())?,
            );
        }
    }
    if !g.is_null() {
        s.guide = GuideSettings {
            enabled: g["enabled"].as_bool().unwrap_or(true),
            allow_http: g["http"].as_bool().unwrap_or(false),
            urls: serde_json::from_value(g["urls"].clone()).unwrap_or_default(),
            history_days: g["history"].as_u64().unwrap_or(7).try_into()?,
            future_hours: g["future"].as_u64().unwrap_or(48).try_into()?,
            language: g["language"].as_str().unwrap_or("it").into(),
            timezone: g["xmlZone"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or("UTC")
                .into(),
            shift_minutes: (g["shift"].as_f64().unwrap_or(0.0) * 60.0).round() as i32,
            archive_timezone: g["archiveZone"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or("Europe/Rome")
                .into(),
            archive_days: g["archiveDays"].as_u64().map(u32::try_from).transpose()?,
            ongoing: g["ongoing"].as_bool().unwrap_or(false),
            before_seconds: g["before"].as_u64().unwrap_or(0).try_into()?,
            after_seconds: g["after"].as_u64().unwrap_or(0).try_into()?,
            ..Default::default()
        };
    }
    s.validate()?;
    Ok(s)
}
fn bounded_name(value: &str, limit: usize) -> String {
    let mut units = 0;
    value
        .chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= limit
        })
        .collect()
}
pub(crate) fn source(s: &Source) -> Value {
    json!({"version":4,"id":s.id,"name":bounded_name(&s.name,100),"location":if s.url.is_some(){"URL"}else{"FILE"},"enabled":s.enabled,"count":s.report.count,"updated":s.updated_at,"checked":s.checked_at,"revision":s.revision,"kind":if s.settings.kind=="radio"{"RADIO"}else{"TV"},"allowHttp":s.settings.allow_http,"error":"","address":s.url.as_deref().unwrap_or(""),"etag":s.etag,"modified":s.modified,"attempted":0,"retryAt":0,"failures":0,"serverRetryAt":0,"complete":true,"importVersion":4,"settings":settings(&s.settings),"classificationVersion":1,"classificationNeedsRefresh":false,"classificationConflicts":0,"documents":s.android_descriptor.get("documents").cloned().unwrap_or_else(||json!({})),"bases":s.android_descriptor.get("bases").cloned().unwrap_or_else(||json!({})),"report":null})
}
pub(crate) fn read_source(h: &Value) -> Result<Source> {
    let id = h["id"]
        .as_str()
        .ok_or_else(|| anyhow!("Lista senza identificativo."))?;
    if !super::store::Library::valid_source(id)
        || !(2..=4).contains(&h["version"].as_u64().unwrap_or(0))
    {
        bail!("Lista del backup non supportata.");
    }
    let name = h["name"].as_str().unwrap_or("");
    if name.trim().is_empty()
        || name.encode_utf16().count() > 100
        || name.chars().any(char::is_control)
    {
        bail!("Nome della lista non valido.");
    }
    if !matches!(h["kind"].as_str(), Some("TV" | "RADIO"))
        || h["enabled"].as_bool().is_none()
        || h["updated"].as_u64().is_none()
    {
        bail!("Impostazioni della lista nel backup non valide.");
    }
    let settings = read_settings(h)?;
    let url = if h["location"] == "URL" {
        let raw = h["address"].as_str().unwrap_or("");
        network::validate_url(raw, true)?;
        Some(raw.into())
    } else if h["location"] == "FILE" && h["address"].as_str().unwrap_or("").is_empty() {
        None
    } else {
        bail!("Sorgente del backup non valida.");
    };
    let count = h["count"]
        .as_u64()
        .ok_or_else(|| anyhow!("Numero di voci non valido."))? as usize;
    if count > MAX_ENTRIES {
        bail!("La lista supera 500.000 voci.");
    }
    let documents = if h["documents"].is_null() {
        json!({})
    } else {
        h["documents"].clone()
    };
    let bases = if h["bases"].is_null() {
        json!({})
    } else {
        h["bases"].clone()
    };
    if documents.as_object().is_none_or(|d| d.len() > 16)
        || bases.as_object().is_none_or(|d| d.len() > 16)
    {
        bail!("Descrittore dei documenti non valido.");
    }
    for attrs in documents.as_object().unwrap().values() {
        let attrs: BTreeMap<String, String> = serde_json::from_value(attrs.clone())?;
        if attrs.len() > 64 || attrs.values().any(|s| s.encode_utf16().count() > 64 * 1024) {
            bail!("Metadati del documento troppo grandi.");
        }
    }
    for base in bases.as_object().unwrap().values() {
        let base = base
            .as_str()
            .ok_or_else(|| anyhow!("Indirizzo del documento non valido."))?;
        // File/content bases are retained as metadata only; resource transports still require public HTTP(S).
        if base.len() > 8192 || base.chars().any(char::is_control) {
            bail!("Indirizzo del documento non valido.");
        }
    }
    let descriptor =
        if documents.as_object().unwrap().is_empty() && bases.as_object().unwrap().is_empty() {
            Value::Null
        } else {
            json!({"documents":documents,"bases":bases})
        };
    Ok(Source {
        android_descriptor: descriptor,
        id: id.into(),
        snapshot: uuid::Uuid::new_v4().to_string(),
        name: name.into(),
        url,
        enabled: h["enabled"].as_bool().unwrap_or(false),
        revision: 1,
        updated_at: h["updated"].as_u64().unwrap_or(0),
        checked_at: 0,
        retry_at: 0,
        etag: String::new(),
        modified: String::new(),
        settings,
        report: ImportReport {
            count,
            ..Default::default()
        },
        guide: Default::default(),
        error: String::new(),
    })
}
fn entry_mime(e: &Entry) -> Option<String> {
    e.metadata
        .properties
        .get("lumetv.mime")
        .or_else(|| e.metadata.properties.get("mimetype"))
        .cloned()
        .or_else(|| {
            e.metadata
                .properties
                .get("inputstream.adaptive.manifest_type")
                .or_else(|| e.metadata.properties.get("inputstream.adaptive.file_type"))
                .and_then(|v| match v.to_lowercase().as_str() {
                    "mpd" | "dash" => Some("application/dash+xml".into()),
                    "hls" | "m3u8" => Some("application/x-mpegURL".into()),
                    _ => None,
                })
        })
}
pub(crate) fn entry(e: &Entry) -> Value {
    let mut attributes = e.metadata.attributes.clone();
    if !e.metadata.tvg_name.is_empty() {
        attributes
            .entry("tvg-name".into())
            .or_insert(e.metadata.tvg_name.clone());
    }
    json!({"name":e.name,"url":e.url,"tvgId":e.metadata.tvg_id,"logo":e.logo,"number":e.number,"groups":e.groups,"kind":if e.kind==crate::model::MediaKind::Radio{"RADIO"}else{"TV"},"headers":android_headers(&e.headers),"localplay":e.metadata.properties.get("lumetv.localplay").map(|v|v.to_uppercase()),"mime":entry_mime(e),"position":e.position,"properties":e.metadata.properties.iter().filter(|(k,_)|!["lumetv.mime","lumetv.localplay","lumetv.vod_marker","lumetv.legacy_vod","lumetv.document"].contains(&k.as_str())).collect::<BTreeMap<_,_>>(),"metadata":{"attributes":attributes,"duration":e.metadata.duration,"document":"root","seekable":null,"content":e.content.to_uppercase(),"vodMarker":if e.metadata.properties.get("lumetv.vod_marker").is_some_and(|v|v=="true"){"ENTRY"}else{"NONE"},"classificationReason":null,"classificationVersion":1,"legacyVod":e.metadata.properties.get("lumetv.legacy_vod").is_some_and(|v|v=="true")}})
}
pub(crate) fn entry_for(e: &Entry, source: &Source) -> Value {
    let mut value = entry(e);
    let document = e
        .metadata
        .properties
        .get("lumetv.document")
        .map(String::as_str)
        .unwrap_or("root");
    if let Some(defaults) = source.android_descriptor["documents"][document].as_object() {
        if let Some(attrs) = value["metadata"]["attributes"].as_object_mut() {
            attrs.retain(|k, v| defaults.get(k) != Some(v));
        }
        value["metadata"]["document"] = json!(document);
    }
    value
}
pub(crate) fn read_entry(v: &Value, h: &Value, s: &Settings, id: &str) -> Result<Entry> {
    let m = &v["metadata"];
    let doc = m["document"].as_str().unwrap_or("root");
    let mut attributes: BTreeMap<String, String> = if h["documents"][doc].is_null() {
        BTreeMap::new()
    } else {
        serde_json::from_value(h["documents"][doc].clone())?
    };
    let own: BTreeMap<String, String> = if m["attributes"].is_null() {
        BTreeMap::new()
    } else {
        serde_json::from_value(m["attributes"].clone())?
    };
    attributes.extend(own);
    if attributes.len() > 128
        || attributes
            .iter()
            .any(|(k, v)| k.len() > 64 * 1024 || v.encode_utf16().count() > 64 * 1024)
    {
        bail!("Metadati della voce troppo grandi.");
    }
    let headers: Headers = if v["headers"].is_null() {
        Headers::new()
    } else {
        serde_json::from_value(v["headers"].clone())?
    };
    network::validate_headers(&headers)?;
    let mut properties: Headers = if v["properties"].is_null() {
        Headers::new()
    } else {
        serde_json::from_value(v["properties"].clone())?
    };
    if properties.len() > 32 {
        bail!("Troppe proprietà della voce.");
    }
    if let Some(mime) = v["mime"].as_str() {
        properties.insert("lumetv.mime".into(), mime.into());
    }
    if let Some(local) = v["localplay"].as_str() {
        properties.insert("lumetv.localplay".into(), local.to_lowercase());
    }
    if m["vodMarker"].as_str().is_some_and(|v| v != "NONE") {
        properties.insert("lumetv.vod_marker".into(), "true".into());
    }
    if m["legacyVod"] == true {
        properties.insert("lumetv.legacy_vod".into(), "true".into());
    }
    if !h["documents"][doc].is_null() {
        properties.insert("lumetv.document".into(), doc.into());
    }
    let base = h["bases"][doc]
        .as_str()
        .and_then(|s| reqwest::Url::parse(s).ok());
    let resolve = |raw: &str| {
        reqwest::Url::parse(raw)
            .or_else(|error| base.as_ref().map(|b| b.join(raw)).unwrap_or(Err(error)))
            .map(|u| u.to_string())
            .unwrap_or_else(|_| raw.to_owned())
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
        if let Some(v) = attributes.get_mut(key) {
            if !v.is_empty() {
                *v = resolve(v);
            }
        }
    }
    for key in ["url-tvg", "x-tvg-url"] {
        if let Some(v) = attributes.get_mut(key) {
            *v = v
                .split([',', ';'])
                .map(|s| resolve(s.trim()))
                .collect::<Vec<_>>()
                .join(",");
        }
    }
    let url = v["url"].as_str().unwrap_or("").to_owned();
    if localplay::read(&url).is_none() {
        network::validate_url(&url, true)?;
    }
    if let Some(route) = localplay::read(&url) {
        properties
            .entry("lumetv.localplay".into())
            .or_insert(route.name.to_lowercase());
    }
    let name = v["name"].as_str().unwrap_or("").to_owned();
    let position = v["position"].as_u64().unwrap_or(u64::MAX);
    let groups: Vec<String> = serde_json::from_value(v["groups"].clone())?;
    if name.trim().is_empty()
        || name.chars().count() > 300
        || position > MAX_ENTRIES as u64
        || groups.len() > 20
        || groups.iter().any(|g| g.chars().count() > 100)
    {
        bail!("Voce della lista non valida.");
    }
    let duration = m["duration"].as_f64().unwrap_or(0.0);
    if !duration.is_finite() {
        bail!("Durata non valida.");
    }
    let mut e = Entry {
        id: id.into(),
        name,
        number: v["number"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| (1..=999999).contains(n)),
        kind: match v["kind"].as_str() {
            Some("TV") => crate::model::MediaKind::Tv,
            Some("RADIO") => crate::model::MediaKind::Radio,
            _ => bail!("Tipo della voce non valido."),
        },
        content: String::new(),
        groups,
        logo: v["logo"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(resolve)
            .unwrap_or_default(),
        url,
        headers,
        metadata: Metadata {
            duration,
            tvg_id: v["tvgId"].as_str().unwrap_or("").into(),
            tvg_name: attributes.get("tvg-name").cloned().unwrap_or_default(),
            guide_urls: ["url-tvg", "x-tvg-url"]
                .iter()
                .filter_map(|k| attributes.get(*k))
                .flat_map(|v| v.split([',', ';']))
                .map(str::to_owned)
                .take(16)
                .collect(),
            shift_hours: attributes
                .get("tvg-shift")
                .and_then(|v| v.parse::<f64>().ok())
                .filter(|v| v.is_finite() && v.abs() <= 24.0)
                .unwrap_or(0.0),
            attributes,
            properties,
            catchup_id: String::new(),
        },
        position: position as usize,
    };
    if m["content"] == "PLAYLIST" {
        bail!("Le inclusioni non sono riproducibili.");
    }
    e.content = super::video::classify(&e, s).into();
    // Saved HTTP entries remain cached when consent is revoked; playback still enforces that consent.
    let mut policy = s.clone();
    policy.allow_http = true;
    localplay::configure(&e, network::Context::new(&policy)?, None)?;
    Ok(e)
}

impl Library {
    pub fn companion_reference(&self, id: &str) -> Result<Value> {
        if self.is_hidden(id) {
            bail!("Il canale è nascosto. Ripristinalo dalle impostazioni.");
        }
        use base64::Engine;
        let (source, selected_entry) = self.entry(id)?;
        let _operation = self.operation.lock().unwrap();
        let current = self.source(&source.id)?;
        if current.snapshot != source.snapshot || current.revision != source.revision {
            bail!("La lista è cambiata. Riapri il contenuto.");
        }
        let cache_path = self.directory.join("companion-fingerprints.json");
        let mut cache: Value = std::fs::read(&cache_path)
            .ok()
            .filter(|b| b.len() < 8192)
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(|| json!({}));
        let signature = hash(&format!(
            "{}:{}:{}",
            source.snapshot,
            source.revision,
            serde_json::to_string(&source.settings)?
        ));
        let fingerprint = if cache[&source.id]["signature"] == signature {
            cache[&source.id]["hash"]
                .as_str()
                .filter(|s| s.len() == 64)
                .map(str::to_owned)
        } else {
            None
        };
        let fingerprint = if let Some(f) = fingerprint {
            f
        } else {
            let mut digest = Sha256::new();
            let mut add = |s: &str| {
                digest.update(s.as_bytes());
                digest.update([0]);
            };
            add("lume-watch-source-v1");
            add(if source.url.is_some() { "URL" } else { "FILE" });
            let mut access = settings(&source.settings)["access"].clone();
            if let Some(a) = access.as_object_mut() {
                a.remove("LOGO");
                a.remove("EPG");
                a.retain(|_, v| {
                    v["headers"].as_object().is_some_and(|h| !h.is_empty())
                        || v["origins"].as_array().is_some_and(|a| !a.is_empty())
                });
                for v in a.values_mut() {
                    if let Some(origins) = v["origins"].as_array_mut() {
                        origins.sort_by_key(|v| v.as_str().unwrap_or("").to_owned());
                    }
                }
            }
            add(&serde_json::to_string(&access)?);
            add(if source.settings.cookies {
                "true"
            } else {
                "false"
            });
            if let Some(url) = &source.url {
                add(url);
            } else {
                let db = super::store::open(&self.path(&source.id)?)?;
                db.execute_batch("PRAGMA temp_store=FILE;CREATE TEMP TABLE companion_identities(value TEXT);BEGIN;")?;
                let mut cursor = 0;
                loop {
                    let mut select=db.prepare("SELECT rowid,kind,payload FROM records WHERE rowid>? AND active=1 ORDER BY rowid LIMIT 200")?;
                    let rows = select
                        .query_map([cursor], |r| {
                            Ok((
                                r.get::<_, i64>(0)?,
                                r.get::<_, String>(1)?,
                                r.get::<_, String>(2)?,
                            ))
                        })?
                        .collect::<std::result::Result<Vec<_>, _>>()?;
                    if rows.is_empty() {
                        break;
                    }
                    for (row, kind, raw) in rows {
                        cursor = row;
                        let e: Entry = serde_json::from_str(&raw)?;
                        let key = base64::engine::general_purpose::STANDARD
                            .encode(Sha256::digest(format!("{}|{}", kind.to_uppercase(), e.url)));
                        db.execute("INSERT INTO companion_identities VALUES(?)", [key])?;
                    }
                }
                db.execute_batch("COMMIT")?;
                let mut rows =
                    db.prepare("SELECT value FROM companion_identities ORDER BY value")?;
                let mut rows = rows.query([])?;
                while let Some(row) = rows.next()? {
                    add(&row.get::<_, String>(0)?);
                }
            }
            let value = format!("{:x}", digest.finalize());
            cache[&source.id] = json!({"signature":signature,"hash":value});
            let temp = cache_path.with_extension("tmp");
            std::fs::write(&temp, serde_json::to_vec(&cache)?)?;
            std::fs::rename(temp, cache_path)?;
            value
        };
        let entry = selected_entry;
        let wire = self::entry(&entry);
        let identity = json!({"url":entry.url,"kind":if entry.kind==crate::model::MediaKind::Radio{"RADIO"}else{"TV"},"headers":wire["headers"],"properties":wire["properties"],"mime":wire["mime"],"localplay":wire["localplay"]});
        let content = entry.display(&source.id, &source.settings).content;
        // Also bind the operation to the local snapshot, without exposing it to the television.
        Ok(
            json!({"snapshot":source.snapshot,"revision":source.revision,"reference":{"builtin":"","source":fingerprint,"lookup":hash(&format!("{}|{}",if entry.kind==crate::model::MediaKind::Radio{"RADIO"}else{"TV"},entry.url)),"entry":hash(&serde_json::to_string(&identity)?),"vod":content=="vod","kind":if entry.kind==crate::model::MediaKind::Radio{"RADIO"}else{"TV"}}}),
        )
    }
    pub fn companion_transfer(
        &self,
        id: &str,
        path: &std::path::Path,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Value> {
        use std::io::Write;
        let _operation = self.operation.lock().unwrap();
        let source = self.source(id)?;
        let db = super::store::open(&self.path(id)?)?;
        let file = std::fs::File::create(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        let mut gzip = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let header =
            json!({"format":"lumetv-playlist","version":1,"metadata":self::source(&source)});
        let bytes = serde_json::to_vec(&header)?;
        gzip.write_all(&bytes)?;
        gzip.write_all(b"\n")?;
        let mut expanded = bytes.len() as u64 + 1;
        let mut select = db.prepare(
            "SELECT id,payload,position FROM records WHERE active=1 ORDER BY position,id",
        )?;
        let mut rows = select.query([])?;
        while let Some(row) = rows.next()? {
            check()?;
            let mut e: Entry = serde_json::from_str(&row.get::<_, String>(1)?)?;
            e.id = row.get(0)?;
            e.position = row.get(2)?;
            e.content = video::classify(&e, &source.settings).into();
            let bytes = serde_json::to_vec(&entry_for(&e, &source))?;
            expanded += bytes.len() as u64 + 1;
            if bytes.len() > 2 * 1024 * 1024 || expanded > MAX_DATABASE {
                bail!("La lista supera il limite di trasferimento.");
            }
            gzip.write_all(&bytes)?;
            gzip.write_all(b"\n")?;
            if gzip.get_ref().metadata()?.len() > MAX_BYTES {
                bail!("La lista compressa supera 256 MiB.");
            }
        }
        let file = gzip.finish()?;
        file.sync_all()?;
        if file.metadata()?.len() > MAX_BYTES {
            bail!("La lista compressa supera 256 MiB.");
        }
        Ok(json!({"name":bounded_name(&source.name,100),"count":source.report.count}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inherited_documents_and_relative_artwork_roundtrip_without_flattening_android_limits() {
        let f = super::super::tests::Fixture::new();
        let id = f.import(
            "#EXTM3U\n#EXTINF:-1,Voce\nhttps://media.example/files/clip.mp4\n",
            None,
        );
        let source = f.library.source(&id).unwrap();
        let mut header = self::source(&source);
        let mut defaults = (0..63)
            .map(|n| (format!("default-{n}"), json!(format!("valore-{n}"))))
            .collect::<serde_json::Map<_, _>>();
        defaults.insert("type".into(), json!("movie"));
        header["documents"] = json!({"included":defaults});
        header["bases"] = json!({"included":"https://lists.example/folder/list.m3u"});
        let source = read_source(&header).unwrap();
        let mut own = (0..63)
            .map(|n| (format!("entry-{n}"), json!(format!("valore-{n}"))))
            .collect::<serde_json::Map<_, _>>();
        own.insert("poster".into(), json!("../cover.jpg"));
        let value = json!({"name":"Una voce","url":"https://media.example/files/clip.mp4","logo":"logos/logo.png","kind":"TV","groups":["Film"],"position":0,"headers":{},"properties":{},"metadata":{"document":"included","attributes":own,"duration":-1,"content":"VOD","vodMarker":"NONE","legacyVod":false}});
        let entry = read_entry(
            &value,
            &header,
            &source.settings,
            "personal_000000000000000000000001",
        )
        .unwrap();
        assert_eq!(entry.content, "vod");
        assert_eq!(entry.metadata.attributes.len(), 128);
        assert_eq!(
            entry.metadata.attributes["poster"],
            "https://lists.example/cover.jpg"
        );
        assert_eq!(entry.logo, "https://lists.example/folder/logos/logo.png");
        let exported = entry_for(&entry, &source);
        assert_eq!(
            exported["metadata"]["attributes"]
                .as_object()
                .unwrap()
                .len(),
            64
        );
        assert_eq!(exported["metadata"]["document"], "included");
        assert!(exported["properties"].as_object().unwrap().is_empty());
        assert_eq!(self::source(&source)["documents"], header["documents"]);
        let again = read_entry(
            &exported,
            &self::source(&source),
            &source.settings,
            &entry.id,
        )
        .unwrap();
        assert_eq!(again.content, "vod");
        assert_eq!(again.metadata.attributes, entry.metadata.attributes);
    }
}
