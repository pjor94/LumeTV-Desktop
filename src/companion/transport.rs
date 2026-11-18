// SPDX-License-Identifier: GPL-3.0-or-later
//! A separate private-LAN TLS transport. Public media policies are never relaxed.
use super::*;
use reqwest::{blocking::Client, redirect::Policy};
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
    DigitallySignedStruct, SignatureScheme,
};
use std::time::Duration;
#[derive(Debug)]
struct Trust {
    expected: Option<String>,
    observed: Arc<Mutex<Option<String>>>,
}
fn tlv<'a>(input: &mut &'a [u8]) -> Option<(u8, &'a [u8], &'a [u8])> {
    let original = *input;
    let tag = *input.first()?;
    let first = *input.get(1)?;
    let (len, offset) = if first < 128 {
        (first as usize, 2)
    } else {
        let n = (first & 127) as usize;
        if n == 0 || n > 4 || input.len() < n + 2 {
            return None;
        }
        let mut len = 0usize;
        for b in &input[2..2 + n] {
            len = len.checked_mul(256)?.checked_add(*b as usize)?;
        }
        (len, n + 2)
    };
    let end = offset.checked_add(len)?;
    let data = input.get(offset..end)?;
    let full = input.get(..end)?;
    *input = original.get(end..)?;
    Some((tag, full, data))
}
pub(super) fn spki(cert: &[u8]) -> Option<&[u8]> {
    let mut cert = cert;
    let (tag, _, mut body) = tlv(&mut cert)?;
    if tag != 0x30 || !cert.is_empty() {
        return None;
    }
    let (tag, _, mut tbs) = tlv(&mut body)?;
    if tag != 0x30 {
        return None;
    }
    if tbs.first() == Some(&0xa0) {
        tlv(&mut tbs)?;
    }
    for _ in 0..5 {
        tlv(&mut tbs)?;
    }
    let (tag, full, _) = tlv(&mut tbs)?;
    (tag == 0x30).then_some(full)
}
impl ServerCertVerifier for Trust {
    fn verify_server_cert(
        &self,
        end: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        let pin = spki(end.as_ref())
            .map(|v| format!("{:x}", sha2::Sha256::digest(v)))
            .ok_or_else(|| rustls::Error::General("TV identity invalid".into()))?;
        let mut seen = self.observed.lock().unwrap();
        if self.expected.as_ref().is_some_and(|v| !same(v, &pin))
            || seen.as_ref().is_some_and(|v| !same(v, &pin))
        {
            return Err(rustls::Error::General("TV identity changed".into()));
        }
        *seen = Some(pin);
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        sig: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            sig,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        sig: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            sig,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
pub(super) struct Connection {
    pub(super) endpoint: Endpoint,
    client: Client,
    observed: Arc<Mutex<Option<String>>>,
    token: Option<String>,
}
impl Connection {
    pub(super) fn new(
        endpoint: Endpoint,
        pin: Option<String>,
        token: Option<String>,
    ) -> Result<Self> {
        endpoint.validate()?;
        Self::build(endpoint, pin, token)
    }
    #[cfg(test)]
    pub(super) fn fixture(port: u16, pin: String, token: String) -> Result<Self> {
        Self::build(
            Endpoint {
                host: "127.0.0.1".into(),
                port,
                id: String::new(),
                name: tv_name(),
            },
            Some(pin),
            Some(token),
        )
    }
    fn build(endpoint: Endpoint, pin: Option<String>, token: Option<String>) -> Result<Self> {
        let observed = Arc::new(Mutex::new(None));
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Trust {
            expected: pin,
            observed: observed.clone(),
        }))
        .with_no_client_auth();
        let client = Client::builder()
            .no_proxy()
            .https_only(true)
            .http1_only()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(4))
            .timeout(Duration::from_secs(8))
            .use_preconfigured_tls(config)
            .build()?;
        Ok(Self {
            endpoint,
            client,
            observed,
            token,
        })
    }
    pub(super) fn pin(&self) -> Result<String> {
        self.observed
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| anyhow!("Identità della TV non verificata."))
    }
    pub(super) fn exchange(&self, path: &str, body: Option<&Value>, auth: bool) -> Result<Value> {
        let url = format!(
            "https://{}:{}{path}",
            self.endpoint.host, self.endpoint.port
        );
        let mut request = if let Some(body) = body {
            let bytes = serde_json::to_vec(body)?;
            if bytes.len() > 32768 {
                bail!("Richiesta alla TV troppo grande.");
            }
            self.client
                .post(&url)
                .header("Content-Type", "application/json")
                .body(bytes)
        } else {
            self.client.get(&url)
        };
        if auth {
            request = request.bearer_auth(
                self.token
                    .as_deref()
                    .ok_or_else(|| anyhow!("Associa prima la TV."))?,
            );
        }
        let response=request.send().map_err(|_|anyhow!("La TV non risponde o la sua identità è cambiata. Controlla la rete e l’associazione."))?;
        let status = response.status();
        let mut bytes = Vec::new();
        response
            .take(32769)
            .read_to_end(&mut bytes)
            .map_err(|_| anyhow!("Risposta della TV interrotta."))?;
        if bytes.len() > 32768 {
            bail!("Risposta della TV troppo grande.");
        }
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| anyhow!("Risposta della TV non valida."))?;
        if !status.is_success() {
            let code = value["error"].as_str().unwrap_or("connection_failed");
            return Err(anyhow!("{}", message(code)));
        }
        Ok(value)
    }
}
