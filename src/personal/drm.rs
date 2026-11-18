// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded Kodi license translation. This module never logs keys or URLs.
use super::{
    network::{merge, validate_headers, validate_url, Context, Resource, SessionPolicy},
    Entry, Headers,
};
use anyhow::{anyhow, bail, Result};
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{io::Read, sync::Arc};

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct LicenseRequest {
    pub get: bool,
    pub challenge: String,
    pub template: Option<String>,
    pub wrappers: Vec<String>,
    pub unwrappers: Vec<String>,
    pub path: Vec<String>,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Drm {
    pub system: String,
    pub url: String,
    pub headers: Headers,
    pub local: Option<Vec<u8>>,
    pub request: LicenseRequest,
}
pub fn query(value: &str) -> Result<Headers> {
    if value.len() > 32768 {
        bail!("Configurazione degli header troppo grande.");
    }
    let mut headers = Headers::new();
    for (key, value) in reqwest::Url::parse(&format!("https://headers.invalid/?{value}"))
        .map_err(|_| anyhow!("Header non validi."))?
        .query_pairs()
    {
        let key = key.trim_start_matches('!');
        if key.eq_ignore_ascii_case("seekable") {
            continue;
        }
        let key = if key.eq_ignore_ascii_case("referrer") {
            "Referer"
        } else if key.eq_ignore_ascii_case("cookies") {
            "Cookie"
        } else {
            key
        };
        merge(&mut headers, &[(key.into(), value.into_owned())].into());
    }
    validate_headers(&headers)?;
    Ok(headers)
}
pub fn json_headers(value: &str) -> Result<Headers> {
    let value: Value =
        serde_json::from_str(value).map_err(|_| anyhow!("Header JSON non validi."))?;
    let mut headers = Headers::new();
    for (k, v) in value
        .as_object()
        .ok_or_else(|| anyhow!("Gli header devono essere un oggetto JSON."))?
    {
        headers.insert(
            k.clone(),
            v.as_str()
                .ok_or_else(|| anyhow!("Gli header devono contenere testo."))?
                .into(),
        );
    }
    validate_headers(&headers)?;
    Ok(headers)
}
fn hex(value: &str) -> Result<Vec<u8>> {
    let value = value.trim().replace('-', "");
    if value.len() != 32 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("ClearKey richiede identificativi e chiavi di 128 bit.");
    }
    (0..32)
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).map_err(Into::into))
        .collect()
}
fn b64(value: &str) -> Result<Vec<u8>> {
    STANDARD
        .decode(value)
        .or_else(|_| URL_SAFE_NO_PAD.decode(value.trim_end_matches('=')))
        .map_err(|_| anyhow!("Dati Base64 non validi."))
}
pub fn clear_keys(raw: &str) -> Result<Vec<u8>> {
    if raw.len() > 32768 {
        bail!("Configurazione ClearKey troppo grande.");
    }
    let decoded;
    let raw = if raw.starts_with("data:application/json;base64,") {
        decoded = String::from_utf8(b64(raw.split_once(',').unwrap().1)?)
            .map_err(|_| anyhow!("ClearKey JSON non valido."))?;
        decoded.as_str()
    } else {
        raw
    };
    let mut keys = std::collections::BTreeMap::new();
    let mut add = |kid: Vec<u8>, key: Vec<u8>| -> Result<()> {
        if kid.len() != 16 || key.len() != 16 {
            bail!("ClearKey richiede chiavi di 128 bit.");
        }
        let id = URL_SAFE_NO_PAD.encode(kid);
        let key = URL_SAFE_NO_PAD.encode(key);
        if keys.get(&id).is_some_and(|old| old != &key) {
            bail!("Chiavi ClearKey in conflitto.");
        }
        keys.insert(id, key);
        if keys.len() > 32 {
            bail!("Sono ammesse fino a 32 chiavi ClearKey.");
        }
        Ok(())
    };
    if raw.trim_start().starts_with('{') {
        let root: Value = serde_json::from_str(raw)
            .map_err(|_| anyhow!("Configurazione ClearKey non valida."))?;
        if let Some(array) = root.get("keys").and_then(Value::as_array) {
            if root
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("temporary")
                != "temporary"
            {
                bail!("ClearKey richiede una licenza temporanea.");
            }
            for key in array {
                if key["kty"] != "oct" {
                    bail!("Tipo di chiave ClearKey non supportato.");
                }
                add(
                    b64(key["kid"].as_str().unwrap_or(""))?,
                    b64(key["k"].as_str().unwrap_or(""))?,
                )?;
            }
        } else {
            for (id, key) in root
                .as_object()
                .ok_or_else(|| anyhow!("ClearKey non valido."))?
            {
                add(hex(id)?, hex(key.as_str().unwrap_or(""))?)?;
            }
        }
    } else {
        for pair in raw.split(',') {
            let (id, key) = pair
                .trim()
                .split_once(':')
                .ok_or_else(|| anyhow!("ClearKey richiede coppie KID:KEY."))?;
            add(hex(id)?, hex(key)?)?;
        }
    }
    if keys.is_empty() {
        bail!("La configurazione ClearKey non contiene chiavi.");
    }
    Ok(serde_json::to_vec(
        &json!({"keys":keys.into_iter().map(|(kid,k)|json!({"kty":"oct","kid":kid,"k":k})).collect::<Vec<_>>(),"type":"temporary"}),
    )?)
}
fn operations(value: Option<&Value>, allowed: &[&str]) -> Result<Vec<String>> {
    let list = value
        .and_then(Value::as_str)
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty() && *v != "none")
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if list.len() > 3 || list.iter().any(|v| !allowed.contains(&v.as_str())) {
        bail!("Trasformazione della licenza non supportata.");
    }
    Ok(list)
}
fn parse(
    entry: &Entry,
    context: &Context,
    common: &Headers,
    hls: bool,
) -> Result<(Option<Drm>, Headers, String)> {
    let p = &entry.metadata.properties;
    let get = |name: &str| {
        p.get(&format!("inputstream.adaptive.{name}"))
            .map(String::as_str)
            .unwrap_or("")
    };
    let supported = ["license_type", "license_key", "drm_legacy", "drm"];
    if p.keys().any(|k| {
        (k.contains("license")
            || k.contains("drm")
            || ["server_certificate", "pre_init_data"]
                .iter()
                .any(|s| k.ends_with(s)))
            && !supported
                .iter()
                .any(|s| k == &format!("inputstream.adaptive.{s}"))
    }) {
        bail!("La voce richiede proprietà DRM non supportate.");
    }
    for name in ["inputstream", "inputstreamaddon"] {
        if p.get(name).is_some_and(|s| s != "inputstream.adaptive") {
            bail!("Componente esterno non supportato.");
        }
    }
    let mut kind = get("license_type").to_owned();
    let mut value = get("license_key").to_owned();
    let mut headers = Headers::new();
    let mut request = LicenseRequest::default();
    let mut key_headers = Headers::new();
    let mut key_parameters = String::new();
    let parts = value.split('|').collect::<Vec<_>>();
    if kind.is_empty()
        && get("drm").is_empty()
        && get("drm_legacy").is_empty()
        && hls
        && parts.len() == 4
        && parts[2..].iter().all(|p| p.is_empty())
    {
        key_parameters = parts[0].trim_start_matches(['?', '&']).into();
        if key_parameters.len() > 8192
            || key_parameters
                .chars()
                .any(|c| c.is_control() || "#|".contains(c))
            || key_parameters.contains("://")
        {
            bail!("Parametri della chiave HLS non validi.");
        }
        merge(&mut key_headers, &query(parts[1])?);
        value.clear();
    }
    if !get("drm_legacy").is_empty() {
        let fields = get("drm_legacy").split('|').collect::<Vec<_>>();
        if fields.len() > 3 {
            bail!("DRM legacy non supportato.");
        }
        kind = fields[0].into();
        value = fields.get(1).unwrap_or(&"").to_string();
        headers = query(fields.get(2).unwrap_or(&""))?;
    }
    if !get("drm").is_empty() {
        let root: Value = serde_json::from_str(get("drm"))
            .map_err(|_| anyhow!("Configurazione DRM JSON non valida."))?;
        let root = root
            .as_object()
            .ok_or_else(|| anyhow!("Configurazione DRM non valida."))?;
        if root.len() != 1 {
            bail!("La selezione fra più sistemi DRM non è supportata.");
        }
        let (system, config) = root.iter().next().unwrap();
        kind = system.clone();
        if config.as_object().is_none_or(|o| {
            o.keys()
                .any(|k| !["license", "priority"].contains(&k.as_str()))
        }) {
            bail!("Configurazione DRM non supportata.");
        }
        let license = config["license"]
            .as_object()
            .ok_or_else(|| anyhow!("Manca la configurazione della licenza."))?;
        if license.keys().any(|k| {
            ![
                "server_url",
                "req_headers",
                "keyids",
                "use_http_get_request",
                "req_params",
                "req_data",
                "wrapper",
                "unwrapper",
                "unwrapper_params",
            ]
            .contains(&k.as_str())
        }) {
            bail!("Opzioni della licenza non supportate.");
        }
        value = if let Some(keys) = license.get("keyids") {
            keys.to_string()
        } else {
            license
                .get("server_url")
                .and_then(Value::as_str)
                .unwrap_or("")
                .into()
        };
        if let Some(params) = license.get("req_params").and_then(Value::as_str) {
            value.push_str(params);
        }
        if let Some(h) = license.get("req_headers") {
            headers = if let Some(s) = h.as_str() {
                query(s)?
            } else {
                json_headers(&h.to_string())?
            };
        }
        request.get = license
            .get("use_http_get_request")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        request.wrappers = operations(license.get("wrapper"), &["base64", "urlenc"])?;
        request.unwrappers = operations(license.get("unwrapper"), &["base64", "json"])?;
        if request.unwrappers.iter().filter(|s| *s == "json").count() > 1 {
            bail!("Estrazione JSON della licenza non supportata.");
        }
        if let Some(params) = license.get("unwrapper_params") {
            if params
                .as_object()
                .is_none_or(|o| o.keys().any(|k| k != "path_data"))
            {
                bail!("Opzioni di estrazione non supportate.");
            }
            request.path = params["path_data"]
                .as_str()
                .unwrap_or("")
                .split('/')
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
        }
        if let Some(encoded) = license.get("req_data").and_then(Value::as_str) {
            let template = String::from_utf8(b64(encoded)?)
                .map_err(|_| anyhow!("Modello della licenza non valido."))?;
            if template.len() > 32768 || template.contains("{CHA-RAW}") && template != "{CHA-RAW}" {
                bail!("Modello della licenza non supportato.");
            }
            let clean = template
                .replace("{CHA-RAW}", "")
                .replace("{CHA-B64}", "")
                .replace("{CHA-B64U}", "");
            if regex::Regex::new(r"\{[A-Z][A-Z0-9-]*\}")
                .unwrap()
                .is_match(&clean)
            {
                bail!("Segnaposto della licenza non supportato.");
            }
            request.template = Some(template);
        }
    }
    if kind.is_empty() && value.is_empty() {
        return Ok((None, key_headers, key_parameters));
    }
    let system = match kind.trim().to_lowercase().trim_start_matches("urn:uuid:") {
        "clearkey" | "org.w3.clearkey" | "e2719d58-a985-b3c9-781a-b030af78d30e" => {
            "org.w3.clearkey"
        }
        "widevine" | "com.widevine.alpha" | "edef8ba9-79d6-4ace-a3c8-27dcd51d21ed" => {
            "com.widevine.alpha"
        }
        _ => bail!("Sistema DRM non supportato: sono disponibili ClearKey e Widevine."),
    };
    if system == "org.w3.clearkey" && hls {
        bail!("ClearKey personale richiede DASH con cifratura CENC.");
    }
    if value.is_empty() {
        bail!("Mancano i dati della licenza DRM.");
    }
    if system == "org.w3.clearkey"
        && !value.starts_with("https://")
        && !value.starts_with("http://")
    {
        return Ok((
            Some(Drm {
                system: system.into(),
                url: String::new(),
                headers: Headers::new(),
                local: Some(clear_keys(&value)?),
                request,
            }),
            key_headers,
            key_parameters,
        ));
    }
    let fields = value.split('|').collect::<Vec<_>>();
    if fields.len() > 4 {
        bail!("Troppi campi nella licenza.");
    }
    if fields.len() >= 3 {
        request = LicenseRequest::default();
        request.get = fields[2].is_empty();
        request.challenge = match fields[2] {
            "" | "R{SSM}" => "raw",
            "b{SSM}" => "base64",
            "B{SSM}" => "base64url",
            _ => bail!("Challenge della licenza non supportato."),
        }
        .into();
        let response = fields.get(3).copied().unwrap_or("");
        if response == "B" {
            request.unwrappers.push("base64".into());
        } else if !["", "R"].contains(&response) {
            let (prefix, path) = if let Some(s) = response.strip_prefix("JB") {
                ("JB", s)
            } else if let Some(s) = response.strip_prefix("BJ") {
                ("BJ", s)
            } else if let Some(s) = response.strip_prefix('J') {
                ("J", s)
            } else {
                bail!("Risposta della licenza non supportata.");
            };
            request.path = path
                .split('/')
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
            request.unwrappers = match prefix {
                "JB" => vec!["json".into(), "base64".into()],
                "BJ" => vec!["base64".into(), "json".into()],
                _ => vec!["json".into()],
            };
        }
    }
    if request.path.len() > 16
        || request.path.iter().any(|s| {
            s.len() > 100
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        })
        || request.unwrappers.iter().any(|s| s == "json") && request.path.is_empty()
    {
        bail!("Percorso JSON della licenza non valido.");
    }
    if request.get && (request.template.is_some() || !request.wrappers.is_empty()) {
        bail!("Una licenza GET non può usare un corpo.");
    }
    let url = validate_url(fields[0], context.settings.allow_http)?;
    let mut inherited = if super::network::origin(&entry.url)? == url.origin().ascii_serialization()
    {
        common.clone()
    } else {
        common
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("user-agent"))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    };
    merge(&mut inherited, &headers);
    if let Some(h) = fields.get(1) {
        merge(&mut inherited, &query(h)?);
    }
    Ok((
        Some(Drm {
            system: system.into(),
            url: url.into(),
            headers: inherited,
            local: None,
            request,
        }),
        key_headers,
        key_parameters,
    ))
}
pub fn policy(entry: &Entry, context: Arc<Context>) -> Result<Arc<SessionPolicy>> {
    validate_url(&entry.url, context.settings.allow_http)?;
    let p = &entry.metadata.properties;
    let value = |name: &str| {
        p.get(&format!("inputstream.adaptive.{name}"))
            .map(String::as_str)
            .unwrap_or("")
    };
    if !["", "hls", "mpd", "dash"].contains(&value("manifest_type")) {
        bail!("Tipo di manifest non supportato.");
    }
    let mut common = entry.headers.clone();
    merge(&mut common, &query(value("common_headers"))?);
    if let Some(h) = p.get("exthttp") {
        merge(&mut common, &json_headers(h)?);
    }
    if let Some(h) = p.get("lumetv.pipe_headers") {
        merge(&mut common, &query(h)?);
    }
    let stream_headers = query(value("stream_headers"))?;
    let mut manifest = common.clone();
    merge(
        &mut manifest,
        &if p.contains_key("inputstream.adaptive.manifest_headers") {
            query(value("manifest_headers"))?
        } else {
            stream_headers.clone()
        },
    );
    let mut media = common.clone();
    merge(&mut media, &stream_headers);
    let hls = value("manifest_type") == "hls"
        || reqwest::Url::parse(&entry.url)?
            .path()
            .to_lowercase()
            .ends_with(".m3u8");
    let (drm, key_headers, key_parameters) = parse(entry, &context, &common, hls)?;
    let mut key = media.clone();
    merge(&mut key, &key_headers);
    Ok(Arc::new(SessionPolicy {
        context,
        root: entry.url.clone(),
        media,
        manifest,
        key,
        key_parameters,
        drm,
    }))
}
fn percent(bytes: &[u8]) -> Vec<u8> {
    let mut result = String::new();
    for &byte in bytes {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            result.push(byte as char);
        } else {
            result.push_str(&format!("%{byte:02X}"));
        }
    }
    result.into_bytes()
}
impl Drm {
    /// Session-only material for Shaka's explicit ClearKey override. Never use
    /// this in catalog rows or diagnostic reports.
    pub fn local_clear_keys(&self) -> Result<Option<Headers>> {
        let Some(local) = self
            .local
            .as_ref()
            .filter(|_| self.system == "org.w3.clearkey")
        else {
            return Ok(None);
        };
        let root: Value = serde_json::from_slice(local)
            .map_err(|_| anyhow!("Configurazione ClearKey non valida."))?;
        let mut keys = Headers::new();
        for key in root["keys"]
            .as_array()
            .ok_or_else(|| anyhow!("Configurazione ClearKey non valida."))?
        {
            let kid = b64(key["kid"].as_str().unwrap_or(""))?;
            let value = b64(key["k"].as_str().unwrap_or(""))?;
            if kid.len() != 16 || value.len() != 16 {
                bail!("ClearKey richiede chiavi di 128 bit.");
            }
            let hex = |bytes: Vec<u8>| {
                bytes
                    .into_iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            };
            keys.insert(hex(kid), hex(value));
        }
        Ok(Some(keys))
    }
    pub fn license(&self, context: &Context, challenge: &[u8]) -> Result<Vec<u8>> {
        context.check()?;
        if let Some(local) = &self.local {
            return Ok(local.clone());
        }
        if challenge.len() > 1024 * 1024 {
            bail!("Challenge della licenza troppo grande.");
        }
        let mut body = match self.request.challenge.as_str() {
            "base64" => STANDARD.encode(challenge).into_bytes(),
            "base64url" => URL_SAFE_NO_PAD.encode(challenge).into_bytes(),
            _ => challenge.to_vec(),
        };
        if let Some(template) = &self.request.template {
            if template != "{CHA-RAW}" {
                body = template
                    .replace("{CHA-B64}", &STANDARD.encode(challenge))
                    .replace("{CHA-B64U}", &URL_SAFE_NO_PAD.encode(challenge))
                    .into_bytes();
            }
        }
        for op in &self.request.wrappers {
            body = match op.as_str() {
                "base64" => STANDARD.encode(&body).into_bytes(),
                "urlenc" => percent(&body),
                _ => unreachable!(),
            };
        }
        let response = context.request(
            &self.url,
            &self.url,
            Resource::License,
            &self.headers,
            if self.request.get {
                Method::GET
            } else {
                Method::POST
            },
            if self.request.get { None } else { Some(body) },
        )?;
        if !response.status().is_success() {
            bail!("Il server della licenza ha rifiutato la richiesta.");
        }
        let mut bytes = Vec::new();
        response
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| anyhow!("Risposta della licenza incompleta."))?;
        if bytes.len() > 1024 * 1024 {
            bail!("Risposta della licenza troppo grande.");
        }
        for op in &self.request.unwrappers {
            bytes = match op.as_str() {
                "base64" => b64(std::str::from_utf8(&bytes)
                    .map_err(|_| anyhow!("Licenza Base64 non valida."))?
                    .trim())?,
                "json" => {
                    let root: Value = serde_json::from_slice(&bytes)
                        .map_err(|_| anyhow!("Risposta JSON della licenza non valida."))?;
                    let mut value = &root;
                    for part in &self.request.path {
                        value = if let Some(a) = value.as_array() {
                            part.parse::<usize>().ok().and_then(|i| a.get(i))
                        } else {
                            value.get(part)
                        }
                        .ok_or_else(|| anyhow!("Campo della licenza assente."))?;
                    }
                    value
                        .as_str()
                        .ok_or_else(|| anyhow!("La licenza JSON deve contenere testo."))?
                        .as_bytes()
                        .to_vec()
                }
                _ => unreachable!(),
            };
        }
        Ok(bytes)
    }
}
