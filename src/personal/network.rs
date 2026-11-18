// SPDX-License-Identifier: GPL-3.0-or-later
use super::{drm::Drm, Headers, Settings};
use anyhow::{anyhow, bail, Result};
use reqwest::{cookie::Jar, Client, Method, Url};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{self, Read},
    sync::OnceLock,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Resource {
    Playlist,
    Manifest,
    Media,
    License,
    Key,
    Logo,
    Guide,
}

pub fn origin(value: &str) -> Result<String> {
    let url = Url::parse(value).map_err(|_| anyhow!("Indirizzo del server non valido."))?;
    // Literal CDN addresses bypass DNS resolution, so validate their address
    // with the same public-IP policy used by PublicDns before allowing them.
    let public_literal = url
        .host_str()
        .unwrap_or_default()
        .trim_matches(['[', ']'])
        .parse::<std::net::IpAddr>()
        .is_ok_and(crate::network::public_ip);
    if !["http", "https"].contains(&url.scheme())
        || !url.username().is_empty()
        || url.password().is_some()
        || !(crate::network::public_host(&url) || public_literal)
        || url.port() == Some(0)
    {
        bail!("L’indirizzo deve usare un server pubblico HTTP o HTTPS.");
    }
    Ok(url.origin().ascii_serialization())
}
pub fn validate_url(value: &str, allow_http: bool) -> Result<Url> {
    if value.len() > 8192 {
        bail!("Indirizzo troppo lungo.");
    }
    let url = Url::parse(value).map_err(|_| anyhow!("Indirizzo non valido."))?;
    origin(value)?;
    if url.fragment().is_some() {
        bail!("I frammenti negli indirizzi non sono supportati.");
    }
    if url.scheme() == "http" && !allow_http {
        bail!("Questa sorgente richiede il consenso HTTP.");
    }
    Ok(url)
}
pub fn validate_headers(headers: &Headers) -> Result<()> {
    if headers.len() > 32
        || headers
            .iter()
            .map(|(k, v)| k.len() + v.len())
            .sum::<usize>()
            > 32768
    {
        bail!("Troppi header: massimo 32 e 32 KiB complessivi.");
    }
    for (key, value) in headers {
        if key.is_empty()
            || key.len() > 128
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
            || value.len() > 4096
            || !value.bytes().all(|b| (32..=126).contains(&b))
            || [
                "host",
                "content-length",
                "range",
                "connection",
                "accept-encoding",
                "transfer-encoding",
                "proxy-authorization",
                "proxy-connection",
            ]
            .contains(&key.to_ascii_lowercase().as_str())
        {
            bail!("Header non valido o riservato al trasporto HTTP.");
        }
    }
    Ok(())
}
pub fn merge(into: &mut Headers, values: &Headers) {
    for (name, value) in values {
        if let Some(old) = into.keys().find(|k| k.eq_ignore_ascii_case(name)).cloned() {
            into.remove(&old);
        }
        into.insert(name.clone(), value.clone());
    }
}

#[cfg(test)]
mod address_tests {
    use super::*;

    #[test]
    fn literal_cdn_addresses_follow_the_same_public_address_policy_as_dns() {
        for url in [
            "https://1.1.1.1/live.m3u8",
            "http://8.8.8.8:25461/live.m3u8",
            "https://[2606:4700:4700::1111]/live.mpd",
        ] {
            assert!(validate_url(url, true).is_ok());
        }
        for url in [
            "https://127.0.0.1/live",
            "https://127.1/live",
            "https://2130706433/live",
            "https://0x7f000001/live",
            "https://10.0.0.1/live",
            "https://172.16.0.1/live",
            "https://192.168.1.1/live",
            "https://169.254.169.254/live",
            "https://100.64.0.1/live",
            "https://198.18.0.1/live",
            "https://192.0.2.1/live",
            "https://224.0.0.1/live",
            "https://[::1]/live",
            "https://[::ffff:127.0.0.1]/live",
            "https://[fc00::1]/live",
            "https://[fe80::1]/live",
            "https://[2001:db8::1]/live",
            "https://[2002:7f00:1::]/live",
            "https://1.1.1.1:0/live",
            "https://user:pass@1.1.1.1/live",
            "ftp://1.1.1.1/live",
            "https://1.1.1.1/live#fragment",
        ] {
            assert!(validate_url(url, true).is_err(), "{url}");
        }
        assert!(validate_url("http://1.1.1.1/live", false).is_err());
    }
}

pub struct Context {
    pub valid: Arc<AtomicBool>,
    guide_valid: Arc<AtomicBool>,
    cancelled: Option<Arc<AtomicBool>>,
    pub settings: Settings,
    clients: BTreeMap<Resource, Client>,
}
impl Context {
    pub fn new(settings: &Settings) -> Result<Arc<Self>> {
        settings.validate()?;
        let mut clients = BTreeMap::new();
        for role in [
            Resource::Playlist,
            Resource::Manifest,
            Resource::Media,
            Resource::License,
            Resource::Key,
            Resource::Logo,
            Resource::Guide,
        ] {
            let mut builder = Client::builder()
                .no_proxy()
                .referer(false)
                .dns_resolver(Arc::new(crate::network::PublicDns))
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .read_timeout(Duration::from_secs(20))
                .timeout(Duration::from_secs(
                    if role == Resource::Playlist || role == Resource::Guide {
                        900
                    } else {
                        45
                    },
                ));
            if settings.cookies {
                builder = builder.cookie_provider(Arc::new(Jar::default()));
            }
            clients.insert(
                role,
                builder
                    .build()
                    .map_err(|_| anyhow!("Connessione alla sorgente non disponibile."))?,
            );
        }
        Ok(Arc::new(Self {
            valid: Arc::new(AtomicBool::new(true)),
            guide_valid: Arc::new(AtomicBool::new(true)),
            cancelled: None,
            settings: settings.clone(),
            clients,
        }))
    }

    #[cfg(test)]
    pub(super) fn fixture(
        settings: &Settings,
        hosts: &[(&str, std::net::SocketAddr)],
    ) -> Arc<Self> {
        let mut context = Self::new(settings).unwrap();
        let target = Arc::get_mut(&mut context).unwrap();
        for role in [
            Resource::Playlist,
            Resource::Manifest,
            Resource::Media,
            Resource::License,
            Resource::Key,
            Resource::Logo,
            Resource::Guide,
        ] {
            let mut builder = Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(5));
            for (host, address) in hosts {
                builder = builder.resolve(host, *address);
            }
            if settings.cookies {
                builder = builder.cookie_provider(Arc::new(Jar::default()));
            }
            target.clients.insert(role, builder.build().unwrap());
        }
        context
    }
    pub fn check(&self) -> Result<()> {
        if !self.valid.load(Ordering::Acquire)
            || self
                .cancelled
                .as_ref()
                .is_some_and(|c| c.load(Ordering::Acquire))
        {
            bail!("La sorgente è stata modificata o disattivata.");
        }
        Ok(())
    }
    fn for_playback(&self, cancelled: Arc<AtomicBool>) -> Arc<Self> {
        Arc::new(Self {
            valid: self.valid.clone(),
            guide_valid: self.guide_valid.clone(),
            cancelled: Some(cancelled),
            settings: self.settings.clone(),
            clients: self.clients.clone(),
        })
    }
    pub fn revoke(&self) {
        self.valid.store(false, Ordering::Release);
        self.guide_valid.store(false, Ordering::Release);
    }
    pub fn revoke_guide(&self) {
        self.guide_valid.store(false, Ordering::Release);
    }
    pub fn check_for(&self, role: Resource) -> Result<()> {
        self.check()?;
        if role == Resource::Guide && !self.guide_valid.load(Ordering::Acquire) {
            bail!("La configurazione della guida è cambiata.");
        }
        Ok(())
    }
    pub fn inherit_media(&mut self, previous: &Self) {
        self.valid = previous.valid.clone();
        for (role, client) in &previous.clients {
            if *role != Resource::Guide {
                self.clients.insert(*role, client.clone());
            }
        }
    }
    pub fn inherit_download(&mut self, previous: &Self) {
        if let Some(client) = previous.clients.get(&Resource::Playlist) {
            self.clients.insert(Resource::Playlist, client.clone());
        }
    }
    pub fn request(
        &self,
        root: &str,
        target: &str,
        role: Resource,
        headers: &Headers,
        method: Method,
        body: Option<Vec<u8>>,
    ) -> Result<NetworkResponse> {
        self.send(root, target, role, headers, method, body, None)
    }
    #[allow(clippy::too_many_arguments)]
    fn send(
        &self,
        root: &str,
        target: &str,
        role: Resource,
        headers: &Headers,
        method: Method,
        body: Option<Vec<u8>>,
        range: Option<&str>,
    ) -> Result<NetworkResponse> {
        let allow = match role {
            Resource::Guide => self.settings.guide.allow_http,
            Resource::Logo => self.settings.allow_http_logos,
            _ => self.settings.allow_http,
        };
        let initial = origin(root)?;
        let access = self.settings.access.get(&role);
        let trusted = access
            .map(|a| {
                a.origins
                    .iter()
                    .filter_map(|o| origin(o).ok())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut url = validate_url(target, allow)?;
        let mut method = method;
        let mut body = body;
        let mut all = headers.clone();
        if let Some(access) = access {
            merge(&mut all, &access.headers);
        }
        validate_headers(&all)?;
        for redirect in 0..=5 {
            self.check_for(role)?;
            validate_url(url.as_str(), allow)?;
            let authorized = url.origin().ascii_serialization() == initial
                || trusted.contains(&url.origin().ascii_serialization());
            let mut request = self.clients[&role].request(method.clone(), url.clone());
            if !all
                .keys()
                .any(|name| name.eq_ignore_ascii_case("User-Agent"))
            {
                request = request.header("User-Agent", crate::providers::USER_AGENT);
            }
            for (name, value) in &all {
                if authorized || name.eq_ignore_ascii_case("User-Agent") {
                    request = request.header(name, value);
                }
            }
            if let Some(body) = &body {
                request = request.body(body.clone());
            }
            if let Some(range) = range {
                request = request.header("Range", range);
            }
            let response = wait(
                &self.valid,
                &self.guide_valid,
                self.cancelled.as_deref(),
                role,
                async { request.send().await },
            )?;
            let response = NetworkResponse {
                inner: response,
                valid: self.valid.clone(),
                guide_valid: self.guide_valid.clone(),
                cancelled: self.cancelled.clone(),
                role,
                pending: Vec::new(),
                offset: 0,
            };
            self.check_for(role)?;
            if ![301, 302, 303, 307, 308].contains(&response.status().as_u16()) {
                return Ok(response);
            }
            if redirect == 5 {
                bail!("Troppi reindirizzamenti della sorgente.");
            }
            let next = response
                .headers()
                .get("Location")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| url.join(v).ok())
                .ok_or_else(|| anyhow!("Reindirizzamento non valido."))?;
            let code = response.status().as_u16();
            if body.is_some()
                && [307, 308].contains(&code)
                && next.origin() != url.origin()
                && !trusted.contains(&next.origin().ascii_serialization())
            {
                bail!("Il reindirizzamento della licenza richiede un server autorizzato.");
            }
            if code == 303 && method != Method::HEAD
                || [301, 302].contains(&code) && method == Method::POST
            {
                method = Method::GET;
                body = None;
            }
            url = next;
        }
        unreachable!()
    }
}

#[derive(Clone)]
pub struct SessionPolicy {
    pub context: Arc<Context>,
    pub root: String,
    pub media: Headers,
    pub manifest: Headers,
    pub key: Headers,
    pub key_parameters: String,
    pub drm: Option<Drm>,
}
impl SessionPolicy {
    pub fn for_playback(&self, cancelled: Arc<AtomicBool>) -> Arc<Self> {
        let mut policy = self.clone();
        policy.context = self.context.for_playback(cancelled);
        Arc::new(policy)
    }
    pub fn key_system(&self) -> Option<&str> {
        self.drm.as_ref().map(|d| d.system.as_str())
    }
    pub fn media_request(
        &self,
        url: &str,
        role: Resource,
        protocol: &Headers,
    ) -> Result<NetworkResponse> {
        let mut url = url.to_owned();
        let mut headers = match role {
            Resource::Manifest => self.manifest.clone(),
            Resource::Key => self.key.clone(),
            _ => self.media.clone(),
        };
        if role == Resource::Key && !self.key_parameters.is_empty() {
            url.push(if url.contains('?') { '&' } else { '?' });
            url.push_str(&self.key_parameters);
        }
        // Range is generated by the player, never supplied by a playlist.
        let range = protocol
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("range"))
            .map(|(_, v)| v.clone());
        for (k, v) in protocol {
            if !k.eq_ignore_ascii_case("range")
                && !headers.keys().any(|name| name.eq_ignore_ascii_case(k))
            {
                merge(&mut headers, &[(k.clone(), v.clone())].into());
            }
        }
        if range.is_some() {
            // Send the validated range through an internal request method.
            return self
                .context
                .range_request(&self.root, &url, role, &headers, range.as_deref());
        }
        self.context
            .request(&self.root, &url, role, &headers, Method::GET, None)
    }
}

impl Context {
    pub fn range_request(
        &self,
        root: &str,
        target: &str,
        role: Resource,
        headers: &Headers,
        range: Option<&str>,
    ) -> Result<NetworkResponse> {
        if let Some(range) = range {
            if range.len() > 128
                || !range.starts_with("bytes=")
                || !range[6..]
                    .bytes()
                    .all(|b| b.is_ascii_digit() || b"-, ".contains(&b))
            {
                bail!("Intervallo del video non valido.");
            }
        }
        self.send(root, target, role, headers, Method::GET, None, range)
    }
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("personal network runtime")
    })
}
fn wait<F, T>(
    valid: &AtomicBool,
    guide: &AtomicBool,
    cancelled: Option<&AtomicBool>,
    role: Resource,
    future: F,
) -> Result<T>
where
    F: std::future::Future<Output = std::result::Result<T, reqwest::Error>>,
{
    runtime().block_on(async {
        let mut future = std::pin::pin!(future);
        loop {
            if !valid.load(Ordering::Acquire)
                || role == Resource::Guide && !guide.load(Ordering::Acquire)
                || cancelled.is_some_and(|c| c.load(Ordering::Acquire))
            {
                bail!("Richiesta della sorgente annullata.");
            }
            match tokio::time::timeout(Duration::from_millis(100), future.as_mut()).await {
                Ok(result) => {
                    return result.map_err(|_| anyhow!("Il server della sorgente non risponde."))
                }
                Err(_) => continue,
            }
        }
    })
}
/// Pull-based body reader. Cancelling a source drops the in-flight HTTP future
/// within 100 ms, including while waiting for headers or a stalled body.
pub struct NetworkResponse {
    inner: reqwest::Response,
    valid: Arc<AtomicBool>,
    guide_valid: Arc<AtomicBool>,
    cancelled: Option<Arc<AtomicBool>>,
    role: Resource,
    pending: Vec<u8>,
    offset: usize,
}
impl NetworkResponse {
    pub fn status(&self) -> reqwest::StatusCode {
        self.inner.status()
    }
    pub fn headers(&self) -> &reqwest::header::HeaderMap {
        self.inner.headers()
    }
    pub fn url(&self) -> &Url {
        self.inner.url()
    }
    pub fn content_length(&self) -> Option<u64> {
        self.inner.content_length()
    }
}
impl Read for NetworkResponse {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        if !self.valid.load(Ordering::Acquire)
            || self.role == Resource::Guide && !self.guide_valid.load(Ordering::Acquire)
            || self
                .cancelled
                .as_ref()
                .is_some_and(|c| c.load(Ordering::Acquire))
        {
            return Err(io::Error::other("source_cancelled"));
        }
        while self.offset == self.pending.len() {
            let chunk = wait(
                &self.valid,
                &self.guide_valid,
                self.cancelled.as_deref(),
                self.role,
                self.inner.chunk(),
            )
            .map_err(|_| io::Error::other("source_read_cancelled"))?;
            let Some(chunk) = chunk else {
                return Ok(0);
            };
            self.pending = chunk.to_vec();
            self.offset = 0;
            if !self.pending.is_empty() {
                break;
            }
        }
        let count = out.len().min(self.pending.len() - self.offset);
        out[..count].copy_from_slice(&self.pending[self.offset..self.offset + count]);
        self.offset += count;
        Ok(count)
    }
}
