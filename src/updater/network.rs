// SPDX-License-Identifier: GPL-3.0-or-later
use super::{policy::Artifact, Error, Result};
use reqwest::blocking::{Client, Response};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Duration,
};

pub struct Network {
    client: Client,
    retry_at: AtomicU64,
}
impl Network {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .user_agent(concat!("LumeTV-Desktop/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .referer(false)
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                let url = attempt.url();
                if attempt.previous().len() < 5 && allowed(url) {
                    attempt.follow()
                } else {
                    attempt.error("update_redirect_refused")
                }
            }))
            .build()
            .map_err(|_| Error::Network)?;
        Ok(Self {
            client,
            retry_at: AtomicU64::new(0),
        })
    }
    pub fn retry_at(&self) -> u64 {
        self.retry_at.load(Ordering::Relaxed)
    }
    fn response(&self, url: &str) -> Result<Response> {
        if !allowed(&reqwest::Url::parse(url).map_err(|_| Error::Metadata)?) {
            return Err(Error::Metadata);
        }
        if self.retry_at() > super::service::now() {
            return Err(Error::RateLimited);
        }
        let response = self
            .client
            .get(url)
            .header(
                "Accept",
                if url.starts_with("https://api.github.com/") {
                    "application/vnd.github+json"
                } else {
                    "application/octet-stream"
                },
            )
            .send()
            .map_err(|_| Error::Network)?;
        if matches!(response.status().as_u16(), 403 | 429) {
            let now = super::service::now();
            let retry = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .map(|v| now.saturating_add(v));
            let reset = response
                .headers()
                .get("x-ratelimit-reset")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok());
            self.retry_at.store(
                retry
                    .into_iter()
                    .chain(reset)
                    .max()
                    .unwrap_or(now + 3600)
                    .clamp(now + 60, now + 86400),
                Ordering::Relaxed,
            );
            return Err(Error::RateLimited);
        }
        if !response.status().is_success() {
            return Err(Error::Network);
        }
        Ok(response)
    }
    pub fn text(&self, url: &str, max: usize) -> Result<Vec<u8>> {
        let mut bytes = vec![];
        self.response(url)?
            .take(max as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Network)?;
        if bytes.len() > max {
            return Err(Error::Metadata);
        }
        Ok(bytes)
    }
    pub fn download(
        &self,
        url: &str,
        artifact: &Artifact,
        directory: &Path,
        cancelled: &AtomicBool,
        progress: impl Fn(u64),
    ) -> Result<PathBuf> {
        let mut response = self.response(url)?;
        if response
            .content_length()
            .is_some_and(|size| size != artifact.size)
        {
            return Err(Error::Checksum);
        }
        fs::create_dir_all(directory).map_err(|_| Error::Disk)?;
        let partial = directory.join(format!("download-{}.part", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&partial)
                .map_err(|_| Error::Disk)?;
            private_file(&output)?;
            copy_verified(&mut response, &mut output, artifact, cancelled, progress)?;
            output.sync_all().map_err(|_| Error::Disk)?;
            drop(output);
            let complete = directory.join(&artifact.name);
            if complete.exists() {
                fs::remove_file(&complete).map_err(|_| Error::Disk)?;
            }
            fs::rename(&partial, &complete).map_err(|_| Error::Disk)?;
            Ok(complete)
        })();
        let _ = fs::remove_file(partial);
        result
    }
}
pub fn allowed(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.fragment().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && matches!(
            url.host_str(),
            Some(
                "api.github.com"
                    | "github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
            )
        )
}
pub fn copy_verified(
    input: &mut impl Read,
    output: &mut impl Write,
    artifact: &Artifact,
    cancelled: &AtomicBool,
    progress: impl Fn(u64),
) -> Result<()> {
    let mut copied = 0u64;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65_536];
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let count = input.read(&mut buffer).map_err(|_| Error::Network)?;
        if count == 0 {
            break;
        }
        copied += count as u64;
        if copied > artifact.size {
            return Err(Error::Checksum);
        }
        hash.update(&buffer[..count]);
        output
            .write_all(&buffer[..count])
            .map_err(|_| Error::Disk)?;
        progress(copied);
    }
    if cancelled.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    if copied != artifact.size || format!("{:x}", hash.finalize()) != artifact.sha256 {
        return Err(Error::Checksum);
    }
    Ok(())
}
pub fn verify_file(path: &Path, artifact: &Artifact) -> Result<()> {
    let mut file = fs::File::open(path).map_err(|_| Error::Disk)?;
    if !file.metadata().map_err(|_| Error::Disk)?.is_file() {
        return Err(Error::Disk);
    }
    copy_verified(
        &mut file,
        &mut std::io::sink(),
        artifact,
        &AtomicBool::new(false),
        |_| {},
    )
}
pub fn private_file(file: &fs::File) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| Error::Disk)?;
    }
    #[cfg(not(unix))]
    let _ = file;
    Ok(())
}
