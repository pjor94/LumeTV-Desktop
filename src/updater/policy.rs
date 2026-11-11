// SPDX-License-Identifier: GPL-3.0-or-later
use super::{Error, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use ring::signature::{UnparsedPublicKey, ED25519};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const REPOSITORY: &str = "pjor94/LumeTV-Desktop";
pub const FEED: &str = "https://api.github.com/repos/pjor94/LumeTV-Desktop/releases?per_page=100";
pub const MAX_PACKAGE: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_MANIFEST: usize = 65_536;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Artifact {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Target {
    pub archive: Artifact,
    pub installer: Option<Artifact>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    pub application_id: String,
    pub repository: String,
    pub version: String,
    pub prerelease: bool,
    #[serde(default)]
    pub notes: String,
    pub targets: BTreeMap<String, Target>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct SignedManifest {
    pub json: String,
    pub signature: String,
}
#[derive(Clone, Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub draft: bool,
    pub prerelease: bool,
    pub assets: Vec<ReleaseAsset>,
}
#[derive(Clone, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub size: u64,
    pub state: String,
    pub browser_download_url: String,
    pub digest: Option<String>,
}

pub fn version(value: &str) -> Result<Version> {
    if value.len() > 100
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
    {
        return Err(Error::Metadata);
    }
    Version::parse(value).map_err(|_| Error::Metadata)
}
pub fn platform() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("macos-arm64"),
        ("macos", "x86_64") => Some("macos-x64"),
        ("windows", "x86_64") => Some("windows-x64"),
        ("linux", "x86_64") => Some("linux-x64"),
        _ => None,
    }
}
pub fn asset_url(release: &str, name: &str) -> Result<String> {
    version(release)?;
    if !matches!(name, "update.json" | "update.sig")
        && ![
            "macos-arm64.zip",
            "macos-arm64.dmg",
            "macos-x64.zip",
            "macos-x64.dmg",
            "windows-x64.zip",
            "linux-x64.tar.gz",
            "linux-x64.deb",
        ]
        .iter()
        .any(|suffix| name == format!("LumeTV-{release}-{suffix}"))
    {
        return Err(Error::Metadata);
    }
    Ok(format!(
        "https://github.com/{REPOSITORY}/releases/download/v{release}/{name}"
    ))
}
pub fn public_key() -> Result<Vec<u8>> {
    hex(
        include_str!("../../resources/update-public-key.hex").trim(),
        32,
    )
}
pub fn hex(value: &str, length: usize) -> Result<Vec<u8>> {
    if value.len() != length * 2 || !value.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::Metadata);
    }
    (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).map_err(|_| Error::Metadata))
        .collect()
}
impl SignedManifest {
    pub fn verify(&self) -> Result<Manifest> {
        self.verify_with_key(&public_key()?)
    }
    pub fn verify_with_key(&self, key: &[u8]) -> Result<Manifest> {
        if self.json.len() > MAX_MANIFEST || self.signature.len() > 1024 {
            return Err(Error::Metadata);
        }
        let signature = STANDARD
            .decode(self.signature.trim())
            .map_err(|_| Error::Signature)?;
        UnparsedPublicKey::new(&ED25519, key)
            .verify(self.json.as_bytes(), &signature)
            .map_err(|_| Error::Signature)?;
        let manifest: Manifest = serde_json::from_str(&self.json).map_err(|_| Error::Metadata)?;
        manifest.validate()?;
        Ok(manifest)
    }
}
impl Manifest {
    pub fn validate(&self) -> Result<()> {
        let parsed = version(&self.version)?;
        if !parsed.pre.is_empty() && !self.prerelease {
            return Err(Error::Metadata);
        }
        if self.schema != 1
            || self.application_id != "it.lumetv.desktop"
            || self.repository != REPOSITORY
            || self.notes.len() > 16_000
        {
            return Err(Error::Metadata);
        }
        for target in ["macos-arm64", "macos-x64", "windows-x64", "linux-x64"] {
            let item = self.targets.get(target).ok_or(Error::Metadata)?;
            item.archive.validate(
                &self.version,
                target,
                if target == "linux-x64" {
                    "tar.gz"
                } else {
                    "zip"
                },
            )?;
            if let Some(installer) = &item.installer {
                installer.validate(
                    &self.version,
                    target,
                    if target.starts_with("macos-") {
                        "dmg"
                    } else if target == "linux-x64" {
                        "deb"
                    } else {
                        return Err(Error::Metadata);
                    },
                )?;
            }
        }
        Ok(())
    }
    pub fn eligible(&self, installed: &str, preview: bool) -> Result<bool> {
        Ok(version(&self.version)? > version(installed)? && (!self.prerelease || preview))
    }
}
impl Artifact {
    fn validate(&self, release: &str, platform: &str, extension: &str) -> Result<()> {
        if self.name != format!("LumeTV-{release}-{platform}.{extension}")
            || self.size == 0
            || self.size > MAX_PACKAGE
            || self.sha256 != self.sha256.to_ascii_lowercase()
        {
            return Err(Error::Metadata);
        }
        hex(&self.sha256, 32)?;
        Ok(())
    }
}
impl Release {
    pub fn asset(&self, name: &str) -> Result<&ReleaseAsset> {
        let mut assets = self
            .assets
            .iter()
            .filter(|a| a.name == name && a.state == "uploaded");
        let asset = assets.next().ok_or(Error::Metadata)?;
        if assets.next().is_some() {
            return Err(Error::Metadata);
        }
        let release = self.tag_name.strip_prefix('v').ok_or(Error::Metadata)?;
        if asset.browser_download_url != asset_url(release, name)?
            || asset.size == 0
            || asset.size > MAX_PACKAGE
        {
            return Err(Error::Metadata);
        }
        Ok(asset)
    }
    pub fn cross_check(&self, manifest: &Manifest, artifact: &Artifact) -> Result<()> {
        let asset = self.asset(&artifact.name)?;
        if self.tag_name != format!("v{}", manifest.version)
            || self.prerelease != manifest.prerelease
            || asset.size != artifact.size
            || asset
                .digest
                .as_ref()
                .is_some_and(|d| d != &format!("sha256:{}", artifact.sha256))
        {
            return Err(Error::Metadata);
        }
        Ok(())
    }
}

pub fn releases(bytes: &[u8], installed: &str, preview: bool) -> Result<Vec<Release>> {
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(Error::Metadata);
    }
    let current = version(installed)?;
    let mut items: Vec<Release> = serde_json::from_slice(bytes).map_err(|_| Error::Metadata)?;
    items.retain(|r| {
        !r.draft
            && (!r.prerelease || preview)
            && r.tag_name
                .strip_prefix('v')
                .and_then(|v| version(v).ok())
                .is_some_and(|v| v > current)
            && r.assets.iter().any(|a| a.name == "update.json")
            && r.assets.iter().any(|a| a.name == "update.sig")
    });
    items.sort_by(|a, b| {
        version(b.tag_name.trim_start_matches('v'))
            .ok()
            .cmp(&version(a.tag_name.trim_start_matches('v')).ok())
    });
    items.truncate(20);
    Ok(items)
}
