// SPDX-License-Identifier: GPL-3.0-or-later
// Broadcaster flows adapted from WorldLiveTV 1.2.14 + hotfix 121417 (WLTV Group),
// through Lume TV's RaiResolver.kt, MediasetResolver.kt, La7Resolver.kt and StreamResolver.kt.
// Modified by the Lume TV project, 2026-11-07: Rust desktop prototype port;
// bounded blocking HTTP and safe errors. Expanded on 2026-11-08 to the full
// Android catalogue, including Sky, Discovery, regional Rai and direct sources.

use crate::model::{Channel, DrmSpec, ResolvedStream};
use anyhow::{anyhow, bail, Context, Result};
use quick_xml::{events::Event, Reader};
use regex::Regex;
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::Url;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::io::Read;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub(crate) const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36";
const MAX_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
const RAI_HOST: &str = "https://www.raiplay.it";
const MEDIASET_HOST: &str = "https://mediasetinfinity.mediaset.it";
const MEDIASET_API: &str = "https://api-ott-prod-fe.mediaset.net";
const LA7_HOST: &str = "https://www.la7play.it";

type Headers = BTreeMap<String, String>;

pub fn channels() -> Vec<Channel> {
    crate::catalog::catalog()
        .iter()
        .map(|c| c.stream_channel())
        .collect()
}

// Never attach a reqwest error as a source: it can contain a token-bearing URL.
#[derive(Debug)]
struct HttpStageError {
    stage: &'static str,
    status: u16,
}

impl fmt::Display for HttpStageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: HTTP {}", self.stage, self.status)
    }
}

impl std::error::Error for HttpStageError {}

#[derive(Clone)]
struct MediasetSession {
    app_name: String,
    sid: String,
    token: String,
    created_at: Instant,
}

impl MediasetSession {
    fn fresh(&self) -> bool {
        self.created_at.elapsed() < Duration::from_secs(15 * 60)
    }
}

pub struct Resolver {
    client: Client,
    rai_services: Mutex<Option<(Instant, Value)>>,
    mediaset_session: Mutex<Option<MediasetSession>>,
}

impl Resolver {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            // Preserve the broadcaster Referer over redirects; never copy a token-bearing URL into it.
            .referer(false)
            .user_agent(USER_AGENT)
            .build()
            .map_err(|_| anyhow!("HTTP: inizializzazione del client non riuscita"))?;
        Ok(Self {
            client,
            rai_services: Mutex::new(None),
            mediaset_session: Mutex::new(None),
        })
    }

    pub fn resolve(&self, channel: &Channel) -> Result<ResolvedStream> {
        match channel.provider.as_str() {
            "rai" | "rainewsrg" => self.resolve_rai(channel),
            "mediaset" => self.resolve_mediaset(channel),
            "la7" => self.resolve_la7(channel),
            "sky" => self.resolve_sky(channel),
            "discovery" => self.resolve_discovery(channel),
            "direct" => {
                let url = http_url(&channel.key, "Diretta")?;
                let mut headers = channel.headers.clone();
                headers
                    .entry("User-Agent".into())
                    .or_insert_with(|| USER_AGENT.into());
                Ok(ResolvedStream {
                    mime_type: if channel.kind == crate::model::MediaKind::Radio
                        && !url.path().ends_with(".m3u8")
                        && !url.path().ends_with(".mpd")
                    {
                        None
                    } else {
                        Some(mime_for_url(url.as_str()).into())
                    },
                    url: url.into(),
                    headers,
                    drm: None,
                })
            }
            _ => bail!("L'emittente non è supportata da questo prototipo"),
        }
    }

    fn resolve_sky(&self, channel: &Channel) -> Result<ResolvedStream> {
        let id = match channel.key.as_str() {
            "Sky TG24" => 1,
            "Cielo" => 2,
            "TV8" => 7,
            _ => bail!("Sky: canale non riconosciuto"),
        };
        let body = self.get(
            &format!("https://video.sky.it/api/v1/getLivestream?id={id}&isMobile=false"),
            &Headers::new(),
            "Sky diretta",
        )?;
        sky_stream(&parse_json(&body, "Sky diretta")?)
    }

    fn resolve_discovery(&self, channel: &Channel) -> Result<ResolvedStream> {
        let (environment, host, default_id) =
            discovery_channel(&channel.key).context("Discovery: canale non riconosciuto")?;
        let mut headers = browser_headers(&format!("https://{host}"));
        let (endpoint, realm, channel_id) = if ["K2", "Frisbee"].contains(&channel.key.as_str()) {
            (
                "https://public.aurora.enhanced.live".into(),
                "it".into(),
                default_id.into(),
            )
        } else {
            let mut url = http_url(
                "https://public.aurora.enhanced.live/site/configurations/",
                "Discovery configurazione",
            )?;
            url.query_pairs_mut()
                .append_pair("include", "default")
                .append_pair("filter[environment]", environment)
                .append_pair("v", "2");
            let body = self.get(url.as_str(), &headers, "Discovery configurazione")?;
            discovery_config(&parse_json(&body, "Discovery configurazione")?)?
        };
        headers.insert(
            "X-Device-Info".into(),
            "STONEJS/1 (Unknown/Unknown; Windows/NT 10.0; Unknown)".into(),
        );
        headers.insert("X-disco-params".into(), format!("realm={realm}"));
        headers.insert("X-disco-client".into(), "WEB:UNKNOWN:wbdatv:2.1.9".into());
        let mut url = http_url(&format!("{endpoint}/token"), "Discovery sessione")?;
        url.query_pairs_mut().append_pair("realm", &realm);
        let body = self.get(url.as_str(), &headers, "Discovery sessione")?;
        let value = parse_json(&body, "Discovery sessione")?;
        let token = required_string(&value["data"]["attributes"], "token", "Discovery sessione")?;
        headers.insert("Authorization".into(), format!("Bearer {token}"));
        let body = self.post(&format!("{endpoint}/playback/v3/channelPlaybackInfo"),&json!({
            "channelId": channel_id,
            "deviceInfo":{"adBlocker":false,"drmSupported":true,"hdrCapabilities":["SDR"],"hwDecodingCapabilities":[],"soundCapabilities":["STEREO"]},
            "wisteriaProperties":{"platform":"desktop","device":{"type":"desktop","browser":{"name":"chrome","version":"146.0.0.0"}}}
        }),&headers,"Discovery autorizzazione")?;
        discovery_stream(&parse_json(&body, "Discovery autorizzazione")?)
    }

    fn get(&self, url: &str, headers: &Headers, stage: &'static str) -> Result<String> {
        self.read(self.client.get(url), headers, stage)
    }

    fn post(
        &self,
        url: &str,
        data: &Value,
        headers: &Headers,
        stage: &'static str,
    ) -> Result<String> {
        self.read(self.client.post(url).json(data), headers, stage)
    }

    fn read(
        &self,
        mut request: RequestBuilder,
        headers: &Headers,
        stage: &'static str,
    ) -> Result<String> {
        request = request.header("Accept", "*/*");
        for (name, value) in headers {
            request = request.header(name.as_str(), value.as_str());
        }
        let response = request
            .send()
            .map_err(|_| anyhow!("{stage}: richiesta non riuscita (rete, timeout o TLS)"))?;
        if !response.status().is_success() {
            return Err(HttpStageError {
                stage,
                status: response.status().as_u16(),
            }
            .into());
        }
        if response
            .content_length()
            .is_some_and(|size| size > MAX_RESPONSE_BYTES)
        {
            bail!("{stage}: risposta troppo grande");
        }
        let mut body = Vec::new();
        response
            .take(MAX_RESPONSE_BYTES + 1)
            .read_to_end(&mut body)
            .map_err(|_| anyhow!("{stage}: lettura della risposta non riuscita"))?;
        if body.len() as u64 > MAX_RESPONSE_BYTES {
            bail!("{stage}: risposta troppo grande");
        }
        String::from_utf8(body).map_err(|_| anyhow!("{stage}: codifica della risposta non valida"))
    }

    fn rai_catalog(&self) -> Result<Value> {
        let mut cache = self
            .rai_services
            .lock()
            .map_err(|_| anyhow!("Rai: cache dei servizi non disponibile"))?;
        if let Some((created_at, catalog)) = cache.as_ref() {
            if created_at.elapsed() < Duration::from_secs(600) {
                return Ok(catalog.clone());
            }
        }
        let body = self.get(
            "https://www.replaytvmhp.rai.it/hbbtv/launcher/RemoteControl/resources/srvConfig.json",
            &Headers::new(),
            "Rai catalogo servizi",
        )?;
        let catalog = parse_json(&body, "Rai catalogo servizi")?;
        *cache = Some((Instant::now(), catalog.clone()));
        Ok(catalog)
    }

    fn resolve_rai(&self, channel: &Channel) -> Result<ResolvedStream> {
        // As in the Android client, a missing/unreachable service catalog falls back to dirette.json.
        let service = self
            .rai_catalog()
            .ok()
            .and_then(|catalog| rai_service(&catalog, &channel.provider, &channel.key));
        let content_url = if let Some(service) = service {
            service
        } else {
            if channel.provider == "rainewsrg" {
                bail!("Rai: edizione regionale non disponibile al momento");
            }
            let body = self.get(
                "https://www.raiplay.it/dirette.json",
                &Headers::new(),
                "Rai dirette",
            )?;
            let direct = parse_json(&body, "Rai dirette")?;
            direct["contents"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|entry| normalize(string(entry, "channel")) == normalize(&channel.key))
                .and_then(|entry| entry["video"]["content_url"].as_str())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow!("Rai: canale non disponibile nelle dirette"))?
                .to_owned()
        };
        let mut relinker = http_url(&content_url, "Rai relinker")?;
        // setQueryParameter replaces an existing parameter in the original implementation.
        replace_query(&mut relinker, "output", "63");
        replace_query(&mut relinker, "forceUserAgent", "raiplayappletv");
        let response = self.get(
            relinker.as_str(),
            &browser_headers(RAI_HOST),
            "Rai relinker",
        )?;
        rai_stream(&parse_json(&response, "Rai relinker")?)
    }

    fn resolve_mediaset(&self, channel: &Channel) -> Result<ResolvedStream> {
        let result = self.resolve_mediaset_once(channel);
        if let Err(error) = result.as_ref() {
            if error
                .downcast_ref::<HttpStageError>()
                .is_some_and(|error| matches!(error.status, 401 | 403))
            {
                *self
                    .mediaset_session
                    .lock()
                    .map_err(|_| anyhow!("Mediaset: sessione non disponibile"))? = None;
                return self.resolve_mediaset_once(channel);
            }
        }
        result
    }

    fn resolve_mediaset_once(&self, channel: &Channel) -> Result<ResolvedStream> {
        let headers = browser_headers(MEDIASET_HOST);
        let mut session_guard = self
            .mediaset_session
            .lock()
            .map_err(|_| anyhow!("Mediaset: sessione non disponibile"))?;
        if !session_guard.as_ref().is_some_and(MediasetSession::fresh) {
            let page = self.get(MEDIASET_HOST, &headers, "Mediaset pagina iniziale")?;
            let app_name = mediaset_app_name(&page)?;
            let response = self.post(
                &format!("{MEDIASET_API}/PROD/play/idm/anonymous/login/v2.0"),
                &json!({"appName": app_name, "client_id": Uuid::new_v4().to_string()}),
                &headers,
                "Mediaset sessione anonima",
            )?;
            let response = parse_json(&response, "Mediaset sessione anonima")?;
            let login = &response["response"];
            let sid = required_string(login, "sid", "Mediaset sessione anonima")?;
            let token = required_string(login, "beToken", "Mediaset sessione anonima")?;
            *session_guard = Some(MediasetSession {
                app_name,
                sid,
                token,
                created_at: Instant::now(),
            });
        }
        let session = session_guard
            .as_ref()
            .ok_or_else(|| anyhow!("Mediaset: sessione non disponibile"))?
            .clone();
        drop(session_guard);
        let code = if let Some(code) = channel.key.strip_prefix('$') {
            code.to_owned()
        } else {
            let stations = self.get(
                "https://feed.entertainment.tv.theplatform.eu/f/PR1GhC/mediaset-prod-all-stations-v2?fields=callSign,title,tuningInstruction&form=cjson&httpError=true",
                &headers,
                "Mediaset elenco canali",
            )?;
            let stations = parse_json(&stations, "Mediaset elenco canali")?;
            stations["entries"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|entry| string(entry, "title").eq_ignore_ascii_case(&channel.key))
                .and_then(|entry| entry["callSign"].as_str())
                .filter(|code| !code.is_empty())
                .ok_or_else(|| anyhow!("Mediaset: canale non presente nel catalogo"))?
                .to_owned()
        };
        let mut check_url = http_url(
            &format!("{MEDIASET_API}/PROD/play/playback/check/v2.0"),
            "Mediaset autorizzazione",
        )?;
        check_url.query_pairs_mut().append_pair("sid", &session.sid);
        let mut authorization_headers = headers.clone();
        authorization_headers.insert("Authorization".into(), format!("Bearer {}", session.token));
        let response = self.post(
            check_url.as_str(),
            &json!({
                "channelCode": code, "streamType": "LIVE", "delivery": "Streaming",
                "createDevice": "true", "overrideAppName": session.app_name,
            }),
            &authorization_headers,
            "Mediaset autorizzazione",
        )?;
        let response = parse_json(&response, "Mediaset autorizzazione")?;
        let selector = response["response"]["mediaSelector"]
            .as_object()
            .ok_or_else(|| {
                anyhow!("Mediaset: diretta non disponibile o autorizzazione diversa richiesta")
            })?;
        let mut selector_url = http_url(
            selector.get("url").and_then(Value::as_str).unwrap_or(""),
            "Mediaset selezione stream",
        )?;
        for (key, value) in selector {
            if key != "url" && key != "auth" {
                let parameter = value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string());
                selector_url.query_pairs_mut().append_pair(key, &parameter);
            }
        }
        selector_url
            .query_pairs_mut()
            .append_pair("auth", &session.token);
        let smil = self.get(selector_url.as_str(), &headers, "Mediaset selezione stream")?;
        let (url, pid) = parse_smil(&smil)?;
        let drm = if let Some(pid) = pid {
            let mut license_url = http_url(
                "https://widevine.entitlement.theplatform.eu/wv/web/ModularDrm/getRawWidevineLicense",
                "Mediaset licenza",
            )?;
            license_url
                .query_pairs_mut()
                .append_pair("releasePid", &pid)
                .append_pair(
                    "account",
                    "http://access.auth.theplatform.com/data/Account/2702976343",
                )
                .append_pair("schema", "1.0")
                .append_pair("token", &session.token);
            let mut license_headers = headers.clone();
            license_headers.insert("Content-Type".into(), "application/octet-stream".into());
            Some(DrmSpec {
                license_url: license_url.into(),
                headers: license_headers,
            })
        } else {
            None
        };
        Ok(ResolvedStream {
            mime_type: Some(mime_for_url(&url).into()),
            url,
            headers,
            drm,
        })
    }

    fn resolve_la7(&self, channel: &Channel) -> Result<ResolvedStream> {
        let path = match channel.key.as_str() {
            "La7" => "/dirette/la7",
            "La7c" => "/dirette/la7-cinema",
            _ => bail!("La7: canale non riconosciuto"),
        };
        let headers = browser_headers(LA7_HOST);
        let page = self.get(&format!("{LA7_HOST}{path}"), &headers, "La7 configurazione")?;
        let config = la7_player_config(&page)?;
        let settings = la7_settings(&config)?;
        let mut token_url = http_url(LA7_HOST, "La7 autorizzazione")?
            .join(&settings.token_endpoint)
            .map_err(|_| anyhow!("La7: indirizzo di autorizzazione non valido"))?;
        token_url = http_url(token_url.as_str(), "La7 autorizzazione")?;
        replace_query(&mut token_url, "id", &settings.content_id);
        replace_query(&mut token_url, "contentType", "LIVE");
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| anyhow!("La7: orologio di sistema non valido"))?
            .as_millis();
        replace_query(&mut token_url, "v", &millis.to_string());
        let response = self.get(token_url.as_str(), &headers, "La7 autorizzazione")?;
        let token = la7_token(&parse_json(&response, "La7 autorizzazione")?)?;
        let mut license_headers = headers.clone();
        license_headers.insert("Content-Type".into(), "application/octet-stream".into());
        license_headers.insert("preAuthorization".into(), token);
        Ok(ResolvedStream {
            url: settings.manifest,
            mime_type: Some("application/dash+xml".into()),
            headers,
            drm: Some(DrmSpec {
                license_url: settings.license_url,
                headers: license_headers,
            }),
        })
    }
}

fn browser_headers(host: &str) -> Headers {
    BTreeMap::from([
        ("User-Agent".into(), USER_AGENT.into()),
        ("Origin".into(), host.into()),
        ("Referer".into(), format!("{host}/")),
    ])
}

fn parse_json(body: &str, stage: &'static str) -> Result<Value> {
    serde_json::from_str(body).map_err(|_| anyhow!("{stage}: risposta JSON non valida"))
}

fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}

fn required_string(value: &Value, key: &str, stage: &'static str) -> Result<String> {
    let value = string(value, key).trim();
    if value.is_empty() {
        bail!("{stage}: risposta incompleta");
    }
    Ok(value.to_owned())
}

fn http_url(value: &str, stage: &'static str) -> Result<Url> {
    let url = Url::parse(value).map_err(|_| anyhow!("{stage}: indirizzo non valido"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        bail!("{stage}: protocollo non supportato");
    }
    Ok(url)
}

fn replace_query(url: &mut Url, key: &str, value: &str) {
    let preserved: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(name, _)| name != key)
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    url.set_query(None);
    url.query_pairs_mut()
        .extend_pairs(preserved)
        .append_pair(key, value);
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
}

fn rai_service(catalog: &Value, provider: &str, key: &str) -> Option<String> {
    let triplet = if provider == "rainewsrg" {
        let region = key.split('$').next()?;
        let id = match region {
            "vda" => 301,
            "piemonte" => 302,
            "liguria" => 303,
            "lombardia" => 304,
            "veneto" => 305,
            "bolzano" => 306,
            "trento" => 307,
            "fvg" => 309,
            "fjk" => 310,
            "emiliaromagna" => 311,
            "toscana" => 312,
            "marche" => 313,
            "umbria" => 314,
            "lazio" => 315,
            "abruzzo" => 316,
            "molise" => 317,
            "campania" => 318,
            "puglia" => 319,
            "basilicata" => 320,
            "calabria" => 321,
            "sardegna" => 322,
            "sicilia" => 323,
            _ => return None,
        };
        Some(format!("0.0.{id}"))
    } else {
        None
    };
    let key = if key == "Rai Radio 2" {
        "Rai Radio 2 Visual"
    } else {
        key
    };
    for editor in catalog["Configuration"]["Editors"]["Editor"].as_array()? {
        for services in editor["Services"].as_array().into_iter().flatten() {
            if string(services, "delivery") != "3" {
                continue;
            }
            for service in services["Service"].as_array().into_iter().flatten() {
                if if let Some(triplet) = &triplet {
                    string(service, "dvbTriplet") == triplet
                } else {
                    normalize(string(service, "name")) == normalize(key)
                } {
                    return service["url"]
                        .as_str()
                        .filter(|url| !url.is_empty())
                        .map(str::to_owned);
                }
            }
        }
    }
    None
}

fn sky_stream(response: &Value) -> Result<ResolvedStream> {
    let url = http_url(string(response, "streaming_url"), "Sky stream")?;
    let mut headers = Headers::new();
    headers.insert("User-Agent".into(), USER_AGENT.into());
    Ok(ResolvedStream {
        mime_type: Some(mime_for_url(url.as_str()).into()),
        url: url.into(),
        headers,
        drm: None,
    })
}

fn discovery_channel(key: &str) -> Option<(&'static str, &'static str, &'static str)> {
    Some(match key {
        "Nove" => ("nove", "nove.tv", "3"),
        "RealTime" => ("realtime", "realtime.it", "2"),
        "DMAX" => ("dmaxit", "dmax.it", "4"),
        "Giallo" => ("giallo", "giallotv.it", "27"),
        "Turbo" => ("motortrend", "discoveryturbo.it", "11"),
        "HGTV" => ("hgtvit", "homegardentv.it", "13"),
        "FoodNetwork" => ("foodnetwork", "foodnetwork.it", "6"),
        "K2" => ("k2", "k2tv.it", "24"),
        "Frisbee" => ("frisbee", "frisbeetv.it", "26"),
        "Discovery" => ("discoverychannelit", "discovery.it", "12"),
        _ => return None,
    })
}

fn discovery_config(response: &Value) -> Result<(String, String, String)> {
    let simulcast = &response["settings"]["site"]["simulcast"];
    let endpoint = required_string(simulcast, "sonicEndpoint", "Discovery configurazione")?;
    let url = http_url(&endpoint, "Discovery configurazione")?;
    let host = url.host_str().unwrap_or_default();
    if url.scheme() != "https"
        || !["enhanced.live", "disco-api.com", "discoveryplus.com"]
            .iter()
            .any(|s| host == *s || host.ends_with(&format!(".{s}")))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port().is_some_and(|p| p != 443)
    {
        bail!("Discovery: servizio non riconosciuto");
    }
    let realm = required_string(simulcast, "sonicRealm", "Discovery configurazione")?;
    let id = simulcast["channelId"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| simulcast["channelId"].as_u64().map(|v| v.to_string()))
        .context("Discovery: canale non disponibile")?;
    if [&realm, &id].iter().any(|s| {
        s.is_empty()
            || s.len() > 100
            || !s
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    }) {
        bail!("Discovery: configurazione non valida");
    }
    Ok((url.as_str().trim_end_matches('/').into(), realm, id))
}

fn discovery_stream(response: &Value) -> Result<ResolvedStream> {
    let streams = response["data"]["attributes"]["streaming"]
        .as_array()
        .context("Discovery: diretta non disponibile al momento")?;
    let compatible = |s: &&Value| {
        ["hls", "dash"].contains(&string(s, "type"))
            && (s["protection"]["drmEnabled"].as_bool() != Some(true)
                || s["protection"]["schemes"]["widevine"]["licenseUrl"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty()))
    };
    let stream = streams
        .iter()
        .filter(compatible)
        .min_by_key(|s| if string(s, "type") == "hls" { 0 } else { 1 })
        .context("Discovery: formato o DRM non disponibile")?;
    let url = http_url(string(stream, "url"), "Discovery stream")?;
    let drm = if stream["protection"]["drmEnabled"] == true {
        let license_url = http_url(
            string(&stream["protection"]["schemes"]["widevine"], "licenseUrl"),
            "Discovery licenza",
        )?;
        if license_url.scheme() != "https" {
            bail!("Discovery: licenza non valida");
        }
        let mut headers = browser_headers("https://www.discoveryplus.com");
        headers.insert("Content-Type".into(), "application/octet-stream".into());
        Some(DrmSpec {
            license_url: license_url.into(),
            headers,
        })
    } else {
        None
    };
    Ok(ResolvedStream {
        url: url.into(),
        mime_type: Some(
            if string(stream, "type") == "dash" {
                "application/dash+xml"
            } else {
                "application/x-mpegURL"
            }
            .into(),
        ),
        headers: browser_headers("https://www.discoveryplus.com"),
        drm,
    })
}

fn mime_for_url(url: &str) -> &'static str {
    if url.contains(".mpd") {
        "application/dash+xml"
    } else {
        "application/x-mpegURL"
    }
}

fn rai_stream(response: &Value) -> Result<ResolvedStream> {
    let url = http_url(response["video"][0].as_str().unwrap_or(""), "Rai stream")?;
    let licenses = response["licence_server_map"]["drmLicenseUrlValues"].as_array();
    let widevine = licenses
        .into_iter()
        .flatten()
        .find(|license| string(license, "drm").eq_ignore_ascii_case("WIDEVINE"));
    if licenses.is_some_and(|licenses| !licenses.is_empty()) && widevine.is_none() {
        bail!("Rai: sistema DRM richiesto non disponibile in questo prototipo");
    }
    let drm = if let Some(license) = widevine {
        let mut license_url = http_url(string(license, "licenceUrl"), "Rai licenza")?;
        let token = license_url
            .query_pairs()
            .find(|(key, _)| key == "Authorization")
            .map(|(_, value)| value.into_owned());
        let mut headers = browser_headers(RAI_HOST);
        headers.insert("Content-Type".into(), "application/octet-stream".into());
        if let Some(token) = token {
            headers.insert("nv-authorizations".into(), token);
        }
        license_url.set_query(None);
        Some(DrmSpec {
            license_url: license_url.into(),
            headers,
        })
    } else {
        None
    };
    Ok(ResolvedStream {
        mime_type: Some(mime_for_url(url.as_str()).into()),
        url: url.into(),
        headers: browser_headers(RAI_HOST),
        drm,
    })
}

fn mediaset_app_name(page: &str) -> Result<String> {
    static APP_NAME: OnceLock<Regex> = OnceLock::new();
    APP_NAME
        .get_or_init(|| {
            Regex::new(r#"["']app-name["']\s+content=["']([^"']+)"#).expect("constant regex")
        })
        .captures(page)
        .and_then(|capture| capture.get(1))
        .map(|name| name.as_str().to_owned())
        .ok_or_else(|| {
            anyhow!("Mediaset: il servizio di accesso è cambiato, serve un aggiornamento")
        })
}

fn parse_smil(xml: &str) -> Result<(String, Option<String>)> {
    static FORBIDDEN: OnceLock<Regex> = OnceLock::new();
    if FORBIDDEN
        .get_or_init(|| Regex::new(r"(?i)<!\s*(DOCTYPE|ENTITY)").expect("constant regex"))
        .is_match(xml)
    {
        bail!("Mediaset: risposta XML non valida");
    }
    let mut reader = Reader::from_str(xml);
    let mut current: Option<(String, bool, Option<String>)> = None;
    let mut chosen: Option<(String, Option<String>)> = None;
    let mut depth = 0usize;
    let mut saw_root = false;
    loop {
        let event = reader
            .read_event()
            .map_err(|_| anyhow!("Mediaset: risposta XML non valida"))?;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(element) | Event::Empty(element) => {
                if depth == 0 {
                    if saw_root {
                        bail!("Mediaset: risposta XML non valida");
                    }
                    saw_root = true;
                }
                if !empty {
                    depth += 1;
                }
                let mut attributes = BTreeMap::new();
                for attribute in element.attributes() {
                    let attribute =
                        attribute.map_err(|_| anyhow!("Mediaset: risposta XML non valida"))?;
                    let value = attribute
                        .decode_and_unescape_value(reader.decoder())
                        .map_err(|_| anyhow!("Mediaset: risposta XML non valida"))?;
                    attributes.insert(attribute.key.as_ref().to_vec(), value.into_owned());
                }
                let name = element.local_name();
                if name.as_ref() == b"ref" && current.is_none() && chosen.is_none() {
                    if let Some(url) = attributes
                        .get(b"src".as_slice())
                        .filter(|url| url.starts_with("http"))
                    {
                        let encrypted = attributes
                            .get(b"security".as_slice())
                            .is_some_and(|security| security == "commonEncryption");
                        current = Some((url.to_owned(), encrypted, None));
                    }
                } else if name.as_ref() == b"param" {
                    if let Some((_, _, pid)) = current.as_mut() {
                        if pid.is_none() {
                            *pid = attributes.get(b"value".as_slice()).and_then(|value| {
                                value.split('|').find_map(|part| {
                                    part.strip_prefix("pid=")
                                        .filter(|pid| !pid.is_empty())
                                        .map(str::to_owned)
                                })
                            });
                        }
                    }
                }
                // Empty references have no parameters. Finalize them in the same way as </ref>.
                if name.as_ref() == b"ref" && empty {
                    finish_smil_ref(&mut current, &mut chosen)?;
                }
            }
            Event::End(element) => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| anyhow!("Mediaset: risposta XML non valida"))?;
                if element.local_name().as_ref() == b"ref" {
                    finish_smil_ref(&mut current, &mut chosen)?;
                }
            }
            Event::DocType(_) => bail!("Mediaset: risposta XML non valida"),
            Event::Eof => break,
            _ => {}
        }
    }
    // read_event reports unmatched closing tags, but an open ref at EOF also must fail.
    if current.is_some() || depth != 0 {
        bail!("Mediaset: risposta XML incompleta");
    }
    chosen.ok_or_else(|| anyhow!("Mediaset: l'emittente non ha autorizzato questa diretta"))
}

fn finish_smil_ref(
    current: &mut Option<(String, bool, Option<String>)>,
    chosen: &mut Option<(String, Option<String>)>,
) -> Result<()> {
    if let Some((url, encrypted, pid)) = current.take() {
        if encrypted && pid.is_none() {
            bail!("Mediaset: autorizzazione DRM incompleta");
        }
        let url = http_url(&url, "Mediaset stream")?.to_string();
        *chosen = Some((url, if encrypted { pid } else { None }));
    }
    Ok(())
}

struct La7Settings {
    manifest: String,
    license_url: String,
    token_endpoint: String,
    content_id: String,
}

fn la7_settings(config: &Value) -> Result<La7Settings> {
    let manifest = http_url(
        string(config, "mainLiveUrl")
            .split('|')
            .next()
            .unwrap_or("")
            .trim(),
        "La7 diretta",
    )?
    .to_string();
    let license = string(config, "drmLicenseServer")
        .split(';')
        .filter_map(|entry| entry.split_once('|'))
        .find(|(scheme, _)| scheme.trim() == "com.widevine.alpha")
        .map(|(_, url)| url.trim())
        .ok_or_else(|| anyhow!("La7: configurazione DRM non disponibile"))?;
    Ok(La7Settings {
        manifest,
        license_url: http_url(license, "La7 licenza")?.to_string(),
        token_endpoint: required_string(config, "drmTokenEndpoint", "La7 autorizzazione")?,
        content_id: required_string(config, "drmContentId", "La7 autorizzazione")?,
    })
}

fn la7_player_config(page: &str) -> Result<Value> {
    static CHUNK: OnceLock<Regex> = OnceLock::new();
    static CONFIGURATION: OnceLock<Regex> = OnceLock::new();
    // Rust's regex engine is linear-time, including on large escaped RSC strings.
    let chunk_regex = CHUNK.get_or_init(|| {
        Regex::new(r#"self\.__next_f\.push\(\s*\[\s*1\s*,\s*("(?:\\.|[^"\\])*")\s*\]\s*\)"#)
            .expect("constant regex")
    });
    let configuration_regex = CONFIGURATION.get_or_init(|| {
        Regex::new(r#""configurazionePlayer"\s*:\s*(\{)"#).expect("constant regex")
    });
    let mut chunks = String::new();
    for capture in chunk_regex.captures_iter(page) {
        let chunk: String = serde_json::from_str(&capture[1])
            .map_err(|_| anyhow!("La7: configurazione non valida"))?;
        chunks.push_str(&chunk);
    }
    for source in [chunks.as_str(), page] {
        if let Some(capture) = configuration_regex.captures(source) {
            let offset = capture.get(1).expect("capture group exists").start();
            let value = serde_json::Deserializer::from_str(&source[offset..])
                .into_iter::<Value>()
                .next()
                .ok_or_else(|| anyhow!("La7: configurazione non valida"))?
                .map_err(|_| anyhow!("La7: configurazione non valida"))?;
            if !value.is_object() {
                bail!("La7: configurazione non valida");
            }
            return Ok(value);
        }
    }
    bail!("La7: la diretta non è disponibile");
}

fn la7_token(response: &Value) -> Result<String> {
    ["preAuthToken", "token", "auth"]
        .into_iter()
        .find_map(|field| {
            response[field]
                .as_str()
                .filter(|value| !value.trim().is_empty())
        })
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("La7: autorizzazione non disponibile"))
}

pub fn probe_manifest(stream: &ResolvedStream) -> Value {
    let result = (|| -> Result<Value> {
        let initial = http_url(&stream.url, "manifest")?;
        if !crate::server::allowed_media(&initial, stream) {
            bail!("manifest_target");
        }
        let trusted = stream.clone();
        let client = Client::builder()
            .no_proxy()
            .dns_resolver(std::sync::Arc::new(crate::network::PublicDns))
            .referer(false)
            .redirect(reqwest::redirect::Policy::custom(move |a| {
                if a.previous().len() < 8 && crate::server::allowed_media(a.url(), &trusted) {
                    a.follow()
                } else {
                    a.stop()
                }
            }))
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| anyhow!("client_init"))?;
        let mut request = client.get(&stream.url);
        for (key, value) in &stream.headers {
            request = request.header(key, value);
        }
        let response = request
            .send()
            .map_err(|_| anyhow!("network_timeout_or_tls"))?;
        let status = response.status().as_u16();
        if !response.status().is_success() {
            return Ok(json!({"status":"failed","http_status":status}));
        }
        let mut bytes = vec![];
        response
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| anyhow!("response_read"))?;
        if bytes.len() > 1024 * 1024 {
            bail!("manifest_too_large");
        }
        let text = String::from_utf8_lossy(&bytes);
        let hls = text
            .trim_start_matches('\u{feff}')
            .trim_start()
            .starts_with("#EXTM3U");
        let dash = text.contains("<MPD") || text.contains(":MPD");
        Ok(
            json!({"status":if hls||dash {"fetched"} else {"invalid_manifest"},"http_status":status,"format":if hls {"hls"} else if dash {"dash"} else {"unknown"},"bytes_read":bytes.len(),"sha256":format!("{:x}",Sha256::digest(&bytes)),"encryption_markers":text.contains("ContentProtection")||text.contains("#EXT-X-KEY"),"license_requested":false,"playback_verified":false}),
        )
    })();
    result.unwrap_or_else(|_|json!({"status":"failed","reason":"network_timeout_tls_or_invalid_response","license_requested":false,"playback_verified":false}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regional_rai_uses_the_correct_triplet_and_never_the_national_rai3_fallback() {
        let catalog = json!({"Configuration":{"Editors":{"Editor":[{"Services":[{"delivery":"3","Service":[
            {"name":"Rai 3","dvbTriplet":"0.0.300","url":"https://media.example/national"},
            {"name":"Rai 3 TGR Lombardia","dvbTriplet":"0.0.304","url":"https://media.example/lombardia"}
        ]}]}]}}});
        assert_eq!(
            rai_service(&catalog, "rainewsrg", "lombardia$live").as_deref(),
            Some("https://media.example/lombardia")
        );
        assert!(rai_service(&catalog, "rainewsrg", "missing").is_none());
        assert!(rai_service(&catalog, "rainewsrg", "lazio").is_none());
    }

    #[test]
    fn discovery_accepts_only_trusted_sonic_endpoints_and_normalizes_numeric_channel_ids() {
        let config = json!({"settings":{"site":{"simulcast":{"sonicEndpoint":"https://eu1-prod.disco-api.com/","sonicRealm":"it","channelId":3}}}});
        assert_eq!(
            discovery_config(&config).unwrap(),
            (
                "https://eu1-prod.disco-api.com".into(),
                "it".into(),
                "3".into()
            )
        );
        for endpoint in [
            "https://evil-discoveryplus.com/",
            "http://eu1-prod.disco-api.com/",
            "https://localhost/",
            "https://token@eu1-prod.disco-api.com/",
        ] {
            let mut value = config.clone();
            value["settings"]["site"]["simulcast"]["sonicEndpoint"] = endpoint.into();
            assert!(discovery_config(&value).is_err());
        }
        assert!(discovery_channel("Nove").is_some());
        assert!(discovery_channel("unknown").is_none());
    }

    #[test]
    fn discovery_prefers_playable_hls_and_uses_widevine_dash_over_fairplay_only_hls() {
        let response = json!({"data":{"attributes":{"streaming":[
            {"type":"hls","url":"https://media.example/fairplay.m3u8","protection":{"drmEnabled":true,"schemes":{"fairplay":{"licenseUrl":"https://license.example/fp"}}}},
            {"type":"dash","url":"https://media.example/protected.mpd","protection":{"drmEnabled":true,"schemes":{"widevine":{"licenseUrl":"https://license.example/wv?opaque=private"}}}}
        ]}}});
        let stream = discovery_stream(&response).unwrap();
        assert_eq!(stream.mime_type.as_deref(), Some("application/dash+xml"));
        assert!(stream.drm.is_some());
        assert!(!stream.headers.contains_key("Authorization"));
        assert!(!stream.drm.unwrap().headers.contains_key("Authorization"));
        let clear = json!({"data":{"attributes":{"streaming":[{"type":"dash","url":"https://media.example/a.mpd"},{"type":"hls","url":"https://media.example/a.m3u8"}]}}});
        let stream = discovery_stream(&clear).unwrap();
        assert_eq!(stream.mime_type.as_deref(), Some("application/x-mpegURL"));
        assert!(stream.drm.is_none());
        assert!(discovery_stream(&json!({"data":{"attributes":{"streaming":[]}}})).is_err());
        assert!(sky_stream(&json!({"streaming_url":"file:///private"})).is_err());
    }

    fn la7_config() -> Value {
        json!({
            "mainLiveUrl": "https://cdn.example/1/live.mpd?a=1&b=2|https://cdn.example/1/live.m3u8",
            "backupLiveUrl": "https://backup.example/wrong.mpd",
            "drmLicenseServer": "com.microsoft.playready|https://license.example/playready;com.widevine.alpha|https://license.example/widevine",
            "drmTokenEndpoint": "https://token.example/tk.php?existing=1",
            "drmContentId": "1",
            "nested": {"description": "A } brace and a \"quote\""},
        })
    }

    #[test]
    fn fixed_catalog_retains_distinct_provider_codes() {
        let catalog = channels();
        assert_eq!(catalog.len(), 126);
        let unique: std::collections::BTreeSet<_> =
            catalog.iter().map(|channel| &channel.id).collect();
        assert_eq!(unique.len(), catalog.len());
        let codes: Vec<_> = catalog
            .iter()
            .filter(|channel| channel.provider == "mediaset")
            .map(|channel| channel.key.as_str())
            .collect();
        for code in ["$C5", "$I1", "$R4"] {
            assert!(codes.contains(&code));
        }
    }

    #[test]
    fn smil_handles_namespaces_and_decodes_url_and_pid() {
        let xml = r#"<smil xmlns="http://www.w3.org/2005/SMIL21/Language"><body><switch><ref src="https://cdn.example/live.mpd?a=1&amp;b=2" security="commonEncryption"><param value="foo|pid=channel-pid|bar"/></ref></switch></body></smil>"#;
        let (url, pid) = parse_smil(xml).unwrap();
        assert_eq!(url, "https://cdn.example/live.mpd?a=1&b=2");
        assert_eq!(pid.as_deref(), Some("channel-pid"));
    }

    #[test]
    fn smil_rejects_encryption_without_pid_and_external_entities() {
        for xml in [
            r#"<smil><ref src="https://cdn.example/live.mpd" security="commonEncryption"/></smil>"#,
            r#"<!DOCTYPE smil [<!ENTITY xxe SYSTEM "file:///etc/passwd">]><smil>&xxe;</smil>"#,
            r#"<smil><ref src="https://cdn.example/live.mpd" security="commonEncryption"><param value="pid="/></ref></smil>"#,
        ] {
            assert!(parse_smil(xml).is_err());
        }
    }

    #[test]
    fn unprotected_smil_does_not_create_a_license_from_tracking_pid() {
        let (_, pid) = parse_smil(r#"<smil><ref src="https://cdn.example/live.m3u8"><param value="pid=tracking-only"/></ref></smil>"#).unwrap();
        assert!(pid.is_none());
    }

    #[test]
    fn rai_service_uses_nested_video_services_and_channel_name() {
        let catalog = json!({"Configuration":{"Editors":{"Editor":[{"Services":[
            {"delivery":"1","Service":[{"name":"Rai 1","url":"https://example/wrong"}]},
            {"delivery":"3","Service":[
                {"name":"Rai 1","url":"https://example/rai1"},
                {"name":"Rai 2","url":"https://example/rai2"},
                {"name":"Rai Radio 2","url":"https://example/audio"},
                {"name":"Rai Radio 2 Visual","url":"https://example/visual"}
            ]}
        ]}]}}});
        assert_eq!(
            rai_service(&catalog, "rai", "rai1").as_deref(),
            Some("https://example/rai1")
        );
        assert_eq!(
            rai_service(&catalog, "rai", "Rai 2").as_deref(),
            Some("https://example/rai2")
        );
        assert_eq!(
            rai_service(&catalog, "rai", "Rai Radio 2").as_deref(),
            Some("https://example/visual")
        );
        assert!(rai_service(&catalog, "rai", "missing").is_none());
    }

    #[test]
    fn rai_license_token_is_separate_from_stream_headers() {
        let stream = rai_stream(&json!({
            "video":["https://cdn.example/live.mpd"],
            "licence_server_map":{"drmLicenseUrlValues":[{"drm":"WIDEVINE","licenceUrl":"https://license.example/wv?Authorization=a%2Bb&other=discard"}]}
        })).unwrap();
        assert!(!stream.headers.contains_key("nv-authorizations"));
        let drm = stream.drm.unwrap();
        assert_eq!(drm.license_url, "https://license.example/wv");
        assert_eq!(
            drm.headers.get("nv-authorizations").map(String::as_str),
            Some("a+b")
        );
    }

    #[test]
    fn rai_rejects_unsupported_drm_instead_of_returning_a_clear_stream() {
        let result = rai_stream(&json!({
            "video":["https://cdn.example/live.mpd"],
            "licence_server_map":{"drmLicenseUrlValues":[{"drm":"FAIRPLAY","licenceUrl":"https://license.example/fps"}]}
        }));
        assert!(result.is_err());
    }

    #[test]
    fn mediaset_app_name_requires_the_expected_configuration() {
        assert_eq!(
            mediaset_app_name(r#"<meta name="app-name" content="mediaset-test">"#).unwrap(),
            "mediaset-test"
        );
        assert!(mediaset_app_name("<html>changed</html>").is_err());
    }

    #[test]
    fn la7_concatenates_rsc_chunks_before_parsing_nested_json() {
        let flight = format!(
            "42:[\"$\",\"Player\",null,{{\"configurazionePlayer\":{}}}]\n",
            la7_config()
        );
        let split = flight.find("configurazionePlayer").unwrap() + 9;
        let page = format!(
            "<script>self.__next_f.push([1,{}])</script><script>self.__next_f.push([0])</script><script>self.__next_f.push([1,{}])</script>",
            serde_json::to_string(&flight[..split]).unwrap(),
            serde_json::to_string(&flight[split..]).unwrap().replace('&', "\\u0026"),
        );
        let parsed = la7_player_config(&page).unwrap();
        let settings = la7_settings(&parsed).unwrap();
        assert_eq!(settings.manifest, "https://cdn.example/1/live.mpd?a=1&b=2");
        assert_eq!(settings.license_url, "https://license.example/widevine");
        assert_eq!(parsed["nested"]["description"], "A } brace and a \"quote\"");
    }

    #[test]
    fn la7_large_escaped_payload_is_parsed_without_recursion_over_strings() {
        let padding = json!("x\"y".repeat(25_000));
        let first = format!("41:[\"$\",\"Placeholder\",null,{{\"padding\":{padding}}}]\n");
        let second = format!(
            "42:[\"$\",\"Player\",null,{{\"configurazionePlayer\":{}}}]\n",
            la7_config()
        );
        let page = format!("<script>self.__next_f.push([1,{}])</script><script>self.__next_f.push([1,{}])</script>", json!(first), json!(second));
        assert!(page.len() > 130_000);
        assert_eq!(
            la7_settings(&la7_player_config(&page).unwrap())
                .unwrap()
                .content_id,
            "1"
        );
    }

    #[test]
    fn la7_requires_complete_settings_before_authorization() {
        for key in [
            "mainLiveUrl",
            "drmLicenseServer",
            "drmTokenEndpoint",
            "drmContentId",
        ] {
            let mut config = la7_config();
            config.as_object_mut().unwrap().remove(key);
            assert!(la7_settings(&config).is_err(), "missing {key}");
        }
        let mut config = la7_config();
        config["drmLicenseServer"] = json!("com.microsoft.playready|https://license.example/pr");
        assert!(la7_settings(&config).is_err());
        assert!(la7_player_config("<html>changed</html>").is_err());
    }

    #[test]
    fn la7_token_variants_require_nonblank_strings() {
        for key in ["preAuthToken", "token", "auth"] {
            let response = Value::Object(serde_json::Map::from_iter([(
                key.into(),
                json!("session-token"),
            )]));
            assert_eq!(la7_token(&response).unwrap(), "session-token");
        }
        for response in [
            json!({}),
            json!({"preAuthToken":""}),
            json!({"preAuthToken":"  "}),
            json!({"preAuthToken":123}),
            json!({"preAuthToken":null}),
        ] {
            assert!(la7_token(&response).is_err());
        }
    }

    #[test]
    fn query_replacement_preserves_unrelated_parameters() {
        let mut url =
            Url::parse("https://token.example/tk.php?existing=1&id=old&id=older").unwrap();
        replace_query(&mut url, "id", "2");
        let pairs: Vec<_> = url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        assert_eq!(
            pairs,
            [("existing".into(), "1".into()), ("id".into(), "2".into())]
        );
    }

    #[test]
    fn failures_never_include_the_supplied_url_or_token() {
        let secret = "secret-token-that-must-not-leak";
        let result = http_url(&format!("file:///tmp/{secret}"), "fixture");
        assert!(!format!("{:#}", result.unwrap_err()).contains(secret));
        let error: anyhow::Error = HttpStageError {
            stage: "fixture",
            status: 403,
        }
        .into();
        assert_eq!(error.to_string(), "fixture: HTTP 403");
    }
}
