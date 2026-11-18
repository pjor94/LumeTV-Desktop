// SPDX-License-Identifier: GPL-3.0-or-later
//! Android's closed Discovery routes. Playlist text can never select a command or arbitrary resolver.
use super::*;
use crate::model::{Channel, MediaKind, ResolvedStream};
use anyhow::{anyhow, bail, Result};
#[derive(Clone, Copy)]
pub(super) struct Route {
    pub name: &'static str,
    pub key: &'static str,
}
const ROUTES: [Route; 10] = [
    Route {
        name: "NOVE",
        key: "Nove",
    },
    Route {
        name: "REAL_TIME",
        key: "RealTime",
    },
    Route {
        name: "DMAX",
        key: "DMAX",
    },
    Route {
        name: "GIALLO",
        key: "Giallo",
    },
    Route {
        name: "MOTOR_TREND",
        key: "Turbo",
    },
    Route {
        name: "HGTV",
        key: "HGTV",
    },
    Route {
        name: "FOOD_NETWORK",
        key: "FoodNetwork",
    },
    Route {
        name: "K2",
        key: "K2",
    },
    Route {
        name: "FRISBEE",
        key: "Frisbee",
    },
    Route {
        name: "DISCOVERY",
        key: "Discovery",
    },
];
pub(super) fn command(value: &str) -> bool {
    value
        .split_once('=')
        .is_some_and(|(key, _)| key.trim().eq_ignore_ascii_case("localplay"))
}
pub(super) fn read(value: &str) -> Option<Route> {
    let (key, value) = value.split_once('=')?;
    if !key.trim().eq_ignore_ascii_case("localplay") {
        return None;
    }
    let value = value.trim().to_ascii_lowercase();
    let value = match value.as_str() {
        "real time" => "realtime",
        "motortrend" | "motor trend" => "turbo",
        "hgtv italia" => "hgtv",
        "food network" => "foodnetwork",
        other => other,
    };
    ROUTES
        .iter()
        .copied()
        .find(|r| r.key.eq_ignore_ascii_case(value))
}
pub(super) fn route(entry: &Entry) -> Result<Option<Route>> {
    if !command(&entry.url) {
        if entry.metadata.properties.contains_key("lumetv.localplay") {
            bail!("Collegamento localplay non valido.");
        }
        return Ok(None);
    }
    let route =
        read(&entry.url).ok_or_else(|| anyhow!("Il collegamento localplay non è supportato."))?;
    if entry
        .metadata
        .properties
        .get("lumetv.localplay")
        .is_some_and(|v| !v.eq_ignore_ascii_case(route.name))
    {
        bail!("Collegamento localplay non coerente.");
    }
    Ok(Some(route))
}
pub(super) fn resolve(entry: &Entry) -> Result<Option<ResolvedStream>> {
    let Some(route) = route(entry)? else {
        return Ok(None);
    };
    crate::providers::Resolver::new()?
        .resolve(&Channel {
            kind: MediaKind::Tv,
            id: entry.id.clone(),
            name: entry.name.clone(),
            provider: "discovery".into(),
            key: route.key.into(),
            headers: Headers::new(),
        })
        .map(Some)
        .map_err(|_| anyhow!("Il servizio del canale non risponde. Riprova fra poco."))
}
pub(super) fn configure(
    entry: &Entry,
    context: Arc<network::Context>,
    base: Option<&ResolvedStream>,
) -> Result<(Entry, Arc<network::SessionPolicy>)> {
    if route(entry)?.is_none() {
        return Ok((entry.clone(), drm::policy(entry, context)?));
    }
    let mut prepared = entry.clone();
    prepared.url = base
        .map(|b| b.url.clone())
        .unwrap_or_else(|| "https://localplay.invalid/validate.m3u8".into());
    let properties = &mut prepared.metadata.properties;
    // A provider supplies current format and protection, ahead of stale static list hints.
    properties.retain(|key, _| {
        !matches!(
            key.as_str(),
            "mimetype" | "inputstream.adaptive.manifest_type" | "inputstream.adaptive.file_type"
        ) && !key.contains("license")
            && !key.contains("drm")
            && !["server_certificate", "pre_init_data"]
                .iter()
                .any(|suffix| key.ends_with(suffix))
    });
    if let Some(base) = base {
        let listed = prepared.headers.clone();
        prepared.headers = base.headers.clone();
        network::merge(&mut prepared.headers, &listed);
        if let Some(mime) = &base.mime_type {
            let kind = if mime.to_lowercase().contains("mpegurl") {
                Some("hls")
            } else if mime.to_lowercase().contains("dash") {
                Some("mpd")
            } else {
                None
            };
            if let Some(kind) = kind {
                properties.insert("inputstream.adaptive.manifest_type".into(), kind.into());
            }
        }
    }
    let mut policy = drm::policy(&prepared, context.clone())?;
    if let Some(drm) = base.and_then(|base| base.drm.as_ref()) {
        network::validate_url(&drm.license_url, context.settings.allow_http)?;
        network::validate_headers(&drm.headers)?;
        Arc::get_mut(&mut policy).expect("new provider policy").drm = Some(drm::Drm {
            system: "com.widevine.alpha".into(),
            url: drm.license_url.clone(),
            headers: drm.headers.clone(),
            local: None,
            request: Default::default(),
        });
    }
    context.check()?;
    Ok((prepared, policy))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_routes_import_and_roundtrip_without_network_or_executable_fallbacks() {
        let f = super::super::tests::Fixture::new();
        let source=f.import("#EXTM3U\n#EXTINF:-1,Nove\n#KODIPROP:inputstream.adaptive.license_type=org.w3.clearkey\n#KODIPROP:inputstream.adaptive.license_key=stale-invalid-hint\n#KODIPROP:inputstream.adaptive.pre_init_data=stale-data\n#KODIPROP:inputstream.adaptive.custom_drm_hint=stale-data\nlocalplay=Nove\n#EXTINF:-1,Real Time\nlocalplay = real time\n#EXTINF:-1,Unsupported\nlocalplay=execute-other-plugin\n",None);
        let rows = f.rows(&source);
        assert_eq!(rows["total"], 2);
        let id = rows["rows"][0]["id"].as_str().unwrap();
        let (s, entry) = f.library.entry(id).unwrap();
        assert_eq!(entry.url, "localplay=Nove");
        let wire = super::super::wire::entry_for(&entry, &s);
        assert_eq!(wire["localplay"], "NOVE");
        let restored = super::super::wire::read_entry(
            &wire,
            &super::super::wire::source(&s),
            &s.settings,
            &entry.id,
        )
        .unwrap();
        assert_eq!(restored.url, entry.url);
        assert!(!rows.to_string().contains("stale-invalid-hint"));
    }
    #[test]
    fn provider_format_and_drm_override_stale_hints_while_headers_keep_android_precedence() {
        let f = super::super::tests::Fixture::new();
        let source=f.import("#EXTM3U\n#EXTINF:-1,Nove\n#EXTVLCOPT:http-user-agent=Listed-UA\n#KODIPROP:inputstream.adaptive.manifest_type=mpd\n#KODIPROP:inputstream.adaptive.license_type=org.w3.clearkey\n#KODIPROP:inputstream.adaptive.license_key=stale-invalid-hint\n#KODIPROP:inputstream.adaptive.pre_init_data=stale-data\n#KODIPROP:inputstream.adaptive.custom_drm_hint=stale-data\nlocalplay=Nove\n",None);
        let id = f.rows(&source)["rows"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let (source, entry) = f.library.entry(&id).unwrap();
        let context = f.library.context(&source).unwrap();
        let base = ResolvedStream {
            url: "https://media.example/live.m3u8".into(),
            mime_type: Some("application/x-mpegURL".into()),
            headers: Headers::from([("User-Agent".into(), "Provider-UA".into())]),
            drm: None,
        };
        let (prepared, policy) = configure(&entry, context.clone(), Some(&base)).unwrap();
        assert_eq!(prepared.url, base.url);
        assert_eq!(policy.manifest["User-Agent"], "Listed-UA");
        assert!(policy.drm.is_none());
        assert_eq!(
            prepared.metadata.properties["inputstream.adaptive.manifest_type"],
            "hls"
        );
        let protected = ResolvedStream {
            url: "https://media.example/live.mpd".into(),
            mime_type: Some("application/dash+xml".into()),
            drm: Some(crate::model::DrmSpec {
                license_url: "https://license.example/widevine".into(),
                headers: Headers::new(),
            }),
            ..base
        };
        let (_, policy) = configure(&entry, context, Some(&protected)).unwrap();
        assert_eq!(policy.key_system(), Some("com.widevine.alpha"));
        assert!(policy.drm.as_ref().unwrap().local.is_none());
    }
}
