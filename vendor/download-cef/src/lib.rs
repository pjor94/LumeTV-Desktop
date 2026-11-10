#![doc = include_str!("../README.md")]

use bzip2::bufread::BzDecoder;
use clap::ValueEnum;
use fs_err::File;
use regex::Regex;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha1_smol::Sha1;
use std::{
    collections::HashMap,
    env,
    fmt::{self, Display},
    io::{self, BufReader, IsTerminal, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    thread,
    time::Duration,
};

#[macro_use]
extern crate thiserror;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Pinned OBS SDK missing or incompatible; automatic CEF SDK fallback is disabled")]
    PinnedSdkRequired,
    #[error("Unsupported target triplet: {0}")]
    UnsupportedTarget(String),
    #[error("HTTP request error: {0}")]
    Request(#[from] ureq::Error),
    #[error("Invalid version: {0}")]
    InvalidVersion(#[from] semver::Error),
    #[error("Version not found: {0}")]
    VersionNotFound(String),
    #[error("Missing Content-Length header")]
    MissingContentLength,
    #[error("Opaque Content-Length header: {0}")]
    OpaqueContentLength(#[from] ureq::http::header::ToStrError),
    #[error("Invalid Content-Length header: {0}")]
    InvalidContentLength(String),
    #[error("File I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Unexpected file size: downloaded {downloaded} expected {expected}")]
    UnexpectedFileSize { downloaded: u64, expected: u64 },
    #[error("Bad SHA1 file hash: {0}")]
    CorruptedFile(String),
    #[error("Invalid archive file path: {0}")]
    InvalidArchiveFile(String),
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error(
        "Undexpected archive version: location: {location} archive {archive} expected {expected}"
    )]
    VersionMismatch {
        location: String,
        archive: String,
        expected: String,
    },
    #[error("Invalid regex pattern: {0}")]
    InvalidRegexPattern(#[from] regex::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

pub const LINUX_TARGETS: &[&str] = &[
    "x86_64-unknown-linux-gnu",
    "arm-unknown-linux-gnueabi",
    "aarch64-unknown-linux-gnu",
];

pub const MACOS_TARGETS: &[&str] = &["aarch64-apple-darwin", "x86_64-apple-darwin"];

pub const WINDOWS_TARGETS: &[&str] = &[
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
    "i686-pc-windows-msvc",
];

pub fn default_version(version: &str) -> String {
    unwrap_cef_version(version).unwrap_or_else(|_| version.to_string())
}

fn unwrap_cef_version(version: &str) -> Result<String> {
    static VERSIONS: OnceLock<Mutex<HashMap<Version, String>>> = OnceLock::new();
    let mut versions = VERSIONS
        .get_or_init(Default::default)
        .lock()
        .expect("Lock error");
    Ok(versions
        .entry(Version::parse(version)?)
        .or_insert_with_key(|v| {
            if v.build.is_empty() {
                version.to_string()
            } else {
                v.build.to_string()
            }
        })
        .clone())
}

pub fn check_archive_json(version: &str, location: &str) -> Result<()> {
    let expected = Version::parse(&unwrap_cef_version(version)?)?;

    static PATTERN: OnceLock<core::result::Result<Regex, regex::Error>> = OnceLock::new();
    let pattern = PATTERN
        .get_or_init(|| Regex::new(r"^cef_binary_([^+]+)(:?\+.+)?$"))
        .as_ref()
        .map_err(Clone::clone)?;
    let archive_json: CefFile = serde_json::from_reader(File::open(archive_json_path(location))?)?;

    // OBS's Windows build has a truthful, non-semver devtools suffix. Check
    // the exact opt-in pins before upstream's semver parser; never relabel it
    // as the binding's version or permit an arbitrary newer CEF build.
    if env::var("LUME_CEF_USE_PINNED_OBS").as_deref() == Ok("1")
        && allow_lume_pinned_sdk(
            &expected,
            &archive_json.name,
            Path::new(location),
            &env::var("TARGET").unwrap_or_else(|_| DEFAULT_TARGET.to_owned()),
        )
    {
        return Ok(());
    }
    let archive_version = pattern.replace(&archive_json.name, "$1");
    let archive = Version::parse(&archive_version)?;

    // Lume's opt-in compatibility exception is restricted to the audited OBS
    // macOS ARM64 SDK patch. archive.json retains the actual SDK version.
    if archive <= expected
        || allow_lume_obs_sdk(
            &expected,
            &archive,
            Path::new(location),
            env::var("LUME_CEF_ALLOW_OBS_150_0_19").as_deref() == Ok("1"),
            cfg!(all(target_os = "macos", target_arch = "aarch64")),
        )
    {
        Ok(())
    } else {
        Err(Error::VersionMismatch {
            location: location.to_string(),
            expected: expected.to_string(),
            archive: archive.to_string(),
        })
    }
}

fn platform_api_hash(header: &str, version: &str, platform: &str) -> Option<String> {
    let marker = format!("#define CEF_API_VERSION_{version} {version}");
    let section = header.split_once(&marker)?.1;
    let section = section.split("#define CEF_API_VERSION_").next()?;
    let section = section.split_once(&format!("defined(OS_{platform})"))?.1;
    let line = section.lines().find(|line| {
        line.trim_start()
            .starts_with(&format!("#define CEF_API_HASH_{version} "))
    })?;
    Some(line.split('"').nth(1)?.to_owned())
}

fn allow_lume_pinned_sdk(
    expected: &Version,
    archive_name: &str,
    location: &Path,
    target: &str,
) -> bool {
    if expected != &Version::new(150, 0, 14) {
        return false;
    }
    let Ok(pins) = serde_json::from_str::<serde_json::Value>(include_str!("../lume-sdk-pins.json"))
    else {
        return false;
    };
    let Some(pin) = pins["targets"].as_object().and_then(|targets| {
        targets
            .values()
            .find(|pin| pin["target"].as_str() == Some(target))
    }) else {
        return false;
    };
    let (Some(version), Some(platform), Some(cef_platform)) = (
        pin["cef_version"].as_str(),
        pin["api_platform"].as_str(),
        pin["cef_platform"].as_str(),
    ) else {
        return false;
    };
    if archive_name != format!("cef_binary_{version}_{cef_platform}.tar.xz") {
        return false;
    }
    let Ok(api) = std::fs::read_to_string(location.join("include/cef_api_versions.h")) else {
        return false;
    };
    let Ok(header) = std::fs::read_to_string(location.join("include/cef_version.h")) else {
        return false;
    };
    api.lines()
        .any(|line| line.trim() == "#define CEF_API_VERSION_LAST CEF_API_VERSION_15000")
        && header
            .lines()
            .any(|line| line.trim() == format!("#define CEF_VERSION \"{version}\""))
        && platform_api_hash(&api, "15000", platform).as_deref() == pin["stable_hash"].as_str()
        && platform_api_hash(&api, "999999", platform).as_deref()
            == pin["experimental_hash"].as_str()
}

// Stable and Experimental hashes from the generated macOS headers of the
// official CEF 150.0.14 SDK and the SHA256-verified OBS 150.0.19 SDK must match.
// See LUME_PATCH.md. This does not enable codecs or alter the CEF/CDM binaries.
const LUME_OBS_MAC_STABLE_HASH: &str = "e85c868aa8a947e1e7c9904c0745c60ae490fa05";
const LUME_OBS_MAC_EXPERIMENTAL_HASH: &str = "0934859000c911cdd75907f63f2ef8e485706e5b";

fn mac_api_hash(header: &str, version: &str) -> Option<String> {
    let start = header.find(&format!("#define CEF_API_VERSION_{version} {version}"))?;
    let section = &header[start..];
    let end = section[1..]
        .find("#define CEF_API_VERSION_")
        .map(|n| n + 1)
        .unwrap_or(section.len());
    let section = &section[..end];
    let mac = section.find("#elif defined(OS_MAC)")?;
    let line = section[mac..].lines().find(|line| {
        line.trim_start()
            .starts_with(&format!("#define CEF_API_HASH_{version} "))
    })?;
    Some(line.split('"').nth(1)?.to_owned())
}

fn allow_lume_obs_sdk(
    expected: &Version,
    archive: &Version,
    location: &Path,
    enabled: bool,
    mac_arm64_host: bool,
) -> bool {
    if !enabled
        || !mac_arm64_host
        || expected != &Version::new(150, 0, 14)
        || archive != &Version::new(150, 0, 19)
    {
        return false;
    }
    let Ok(api_header) = std::fs::read_to_string(location.join("include/cef_api_versions.h"))
    else {
        return false;
    };
    let Ok(version_header) = std::fs::read_to_string(location.join("include/cef_version.h")) else {
        return false;
    };
    let last_is_15000 = api_header
        .lines()
        .any(|line| line.trim() == "#define CEF_API_VERSION_LAST CEF_API_VERSION_15000");
    last_is_15000
        && mac_api_hash(&api_header, "15000").as_deref() == Some(LUME_OBS_MAC_STABLE_HASH)
        && mac_api_hash(&api_header, "999999").as_deref() == Some(LUME_OBS_MAC_EXPERIMENTAL_HASH)
        && version_header.lines().any(|line| {
            line.trim() == "#define CEF_VERSION \"150.0.19+gcf60f42+chromium-150.0.7871.252\""
        })
}

fn archive_json_path<P>(location: P) -> PathBuf
where
    P: AsRef<Path>,
{
    let location = location.as_ref().join("archive.json");
    std::path::absolute(&location).unwrap_or(location)
}

pub const DEFAULT_CDN_URL: &str = "https://cef-builds.spotifycdn.com";

pub fn default_download_url() -> String {
    env::var("CEF_DOWNLOAD_URL").unwrap_or(DEFAULT_CDN_URL.to_owned())
}

#[derive(Clone, PartialEq, Eq, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Stable,
    Beta,
}

impl Display for Channel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Channel::Stable => write!(f, "stable"),
            Channel::Beta => write!(f, "beta"),
        }
    }
}

#[derive(Deserialize, Serialize, Default)]
pub struct CefIndex {
    pub macosarm64: CefPlatform,
    pub macosx64: CefPlatform,
    pub windows64: CefPlatform,
    pub windowsarm64: CefPlatform,
    pub windows32: CefPlatform,
    pub linux64: CefPlatform,
    pub linuxarm64: CefPlatform,
    pub linuxarm: CefPlatform,
}

impl CefIndex {
    pub fn download() -> Result<Self> {
        Self::download_from(DEFAULT_CDN_URL)
    }

    pub fn download_from(url: &str) -> Result<Self> {
        if env::var("LUME_CEF_USE_PINNED_OBS").as_deref() == Ok("1") {
            return Err(Error::PinnedSdkRequired);
        }
        Ok(ureq::get(&format!("{url}/index.json"))
            .call()?
            .into_body()
            .read_json()?)
    }

    pub fn platform(&self, target: &str) -> Result<&CefPlatform> {
        match target {
            "aarch64-apple-darwin" => Ok(&self.macosarm64),
            "x86_64-apple-darwin" => Ok(&self.macosx64),
            "x86_64-pc-windows-msvc" => Ok(&self.windows64),
            "aarch64-pc-windows-msvc" => Ok(&self.windowsarm64),
            "i686-pc-windows-msvc" => Ok(&self.windows32),
            "x86_64-unknown-linux-gnu" => Ok(&self.linux64),
            "aarch64-unknown-linux-gnu" => Ok(&self.linuxarm64),
            "arm-unknown-linux-gnueabi" => Ok(&self.linuxarm),
            v => Err(Error::UnsupportedTarget(v.to_string())),
        }
    }
}

#[derive(Deserialize, Serialize, Default)]
pub struct CefPlatform {
    pub versions: Vec<CefVersion>,
}

impl CefPlatform {
    pub fn version(&self, cef_version: &str) -> Result<&CefVersion> {
        let version_prefix = format!("{cef_version}+");
        self.versions
            .iter()
            .find(|v| v.cef_version.starts_with(&version_prefix))
            .ok_or_else(|| Error::VersionNotFound(cef_version.to_string()))
    }

    pub fn latest(&self, channel: Channel) -> Result<&CefVersion> {
        static PATTERN: OnceLock<core::result::Result<Regex, regex::Error>> = OnceLock::new();
        let pattern = PATTERN
            .get_or_init(|| Regex::new(r"^([^+]+)(:?\+.+)?$"))
            .as_ref()
            .map_err(Clone::clone)?;

        self.versions
            .iter()
            .filter_map(|value| {
                if value.channel == channel {
                    let key = Version::parse(&pattern.replace(&value.cef_version, "$1")).ok()?;
                    Some((key, value))
                } else {
                    None
                }
            })
            .max_by(|(a, _), (b, _)| a.cmp(b))
            .map(|(_, v)| v)
            .ok_or_else(|| Error::VersionNotFound("latest".to_string()))
    }
}

#[derive(Deserialize, Serialize)]
pub struct CefVersion {
    pub channel: Channel,
    pub cef_version: String,
    pub files: Vec<CefFile>,
}

impl CefVersion {
    pub fn download_archive<P>(&self, location: P, show_progress: bool) -> Result<PathBuf>
    where
        P: AsRef<Path>,
    {
        self.download_archive_from(DEFAULT_CDN_URL, location, show_progress)
    }

    pub fn download_archive_from<P>(
        &self,
        url: &str,
        location: P,
        show_progress: bool,
    ) -> Result<PathBuf>
    where
        P: AsRef<Path>,
    {
        let file = self.minimal()?;
        let (file, sha) = (file.name.as_str(), file.sha1.as_str());

        fs_err::create_dir_all(&location)?;
        let download_file = location.as_ref().join(file);

        if download_file.exists() {
            if calculate_file_sha1(&download_file) == sha {
                if show_progress {
                    println!("Verified archive: {}", download_file.display());
                }
                return Ok(download_file);
            }

            if show_progress {
                println!("Cleaning corrupted archive: {}", download_file.display());
            }
            let corrupted_file = location.as_ref().join(format!("corrupted_{file}"));
            fs_err::rename(&download_file, &corrupted_file)?;
            fs_err::remove_file(&corrupted_file)?;
        }

        let cef_url = format!("{url}/{file}");
        if show_progress {
            println!("Using archive url: {cef_url}");
        }

        let mut file = File::create(&download_file)?;

        let resp = ureq::get(&cef_url).call()?;
        let expected = resp
            .headers()
            .get("Content-Length")
            .ok_or(Error::MissingContentLength)?;
        let expected = expected.to_str()?;
        let expected = expected
            .parse::<u64>()
            .map_err(|_| Error::InvalidContentLength(expected.to_owned()))?;

        let downloaded = if show_progress && io::stdout().is_terminal() {
            const DOWNLOAD_TEMPLATE: &str = "{msg} {spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})";

            let bar = indicatif::ProgressBar::new(expected);
            bar.set_style(
                indicatif::ProgressStyle::with_template(DOWNLOAD_TEMPLATE)
                    .expect("invalid template")
                    .progress_chars("##-"),
            );
            bar.set_message("Downloading");
            std::io::copy(
                &mut bar.wrap_read(resp.into_body().into_reader()),
                &mut file,
            )
        } else {
            let mut reader = resp.into_body().into_reader();
            std::io::copy(&mut reader, &mut file)
        }?;

        if downloaded != expected {
            return Err(Error::UnexpectedFileSize {
                downloaded,
                expected,
            });
        }

        if show_progress {
            println!("Verifying SHA1 hash: {sha}...");
        }
        if calculate_file_sha1(&download_file) != sha {
            return Err(Error::CorruptedFile(download_file.display().to_string()));
        }

        if show_progress {
            println!("Downloaded archive: {}", download_file.display());
        }
        Ok(download_file)
    }

    pub fn download_archive_with_retry<P>(
        &self,
        location: P,
        show_progress: bool,
        retry_delay: Duration,
        max_retries: u32,
    ) -> Result<PathBuf>
    where
        P: AsRef<Path>,
    {
        self.download_archive_with_retry_from(
            DEFAULT_CDN_URL,
            location,
            show_progress,
            retry_delay,
            max_retries,
        )
    }

    pub fn download_archive_with_retry_from<P>(
        &self,
        url: &str,
        location: P,
        show_progress: bool,
        retry_delay: Duration,
        max_retries: u32,
    ) -> Result<PathBuf>
    where
        P: AsRef<Path>,
    {
        let mut result = self.download_archive_from(url, &location, show_progress);

        let mut retry = 0;
        while let Err(Error::Io(_)) = &result {
            if retry >= max_retries {
                break;
            }

            retry += 1;
            thread::sleep(retry_delay * retry);

            result = self.download_archive_from(url, &location, show_progress);
        }

        result
    }

    pub fn minimal(&self) -> Result<&CefFile> {
        self.files
            .iter()
            .find(|f| f.file_type == "minimal")
            .ok_or_else(|| Error::VersionNotFound(self.cef_version.clone()))
    }

    pub fn write_archive_json<P>(&self, location: P) -> Result<()>
    where
        P: AsRef<Path>,
    {
        self.minimal()?.write_archive_json(location)
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct CefFile {
    #[serde(rename = "type")]
    pub file_type: String,
    pub name: String,
    pub sha1: String,
}

impl CefFile {
    pub fn write_archive_json<P>(&self, location: P) -> Result<()>
    where
        P: AsRef<Path>,
    {
        let archive_version = serde_json::to_string_pretty(self)?;
        let mut archive_json = File::create(archive_json_path(location))?;
        archive_json.write_all(archive_version.as_bytes())?;
        Ok(())
    }
}

impl TryFrom<&Path> for CefFile {
    type Error = Error;

    fn try_from(location: &Path) -> Result<Self> {
        let file_type = "minimal".to_string();
        let name = location
            .file_name()
            .map(|f| f.display().to_string())
            .ok_or_else(|| Error::InvalidArchiveFile(location.display().to_string()))?;
        let sha1 = calculate_file_sha1(location);
        Ok(Self {
            file_type,
            name,
            sha1,
        })
    }
}

pub fn download_target_archive<P>(
    target: &str,
    cef_version: &str,
    location: P,
    show_progress: bool,
) -> Result<PathBuf>
where
    P: AsRef<Path>,
{
    download_target_archive_from(
        DEFAULT_CDN_URL,
        target,
        cef_version,
        location,
        show_progress,
    )
}

pub fn download_target_archive_from<P>(
    url: &str,
    target: &str,
    cef_version: &str,
    location: P,
    show_progress: bool,
) -> Result<PathBuf>
where
    P: AsRef<Path>,
{
    if show_progress {
        println!("Downloading CEF archive for {target}...");
    }

    let index = CefIndex::download_from(url)?;
    let platform = index.platform(target)?;
    let version = platform.version(cef_version)?;

    version.download_archive_with_retry_from(
        url,
        location,
        show_progress,
        Duration::from_secs(15),
        3,
    )
}

pub fn extract_target_archive<P, Q>(
    target: &str,
    archive: P,
    location: Q,
    show_progress: bool,
) -> Result<PathBuf>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
{
    if show_progress {
        println!("Extracting archive: {}", archive.as_ref().display());
    }
    let decoder = BzDecoder::new(BufReader::new(File::open(archive.as_ref())?));
    tar::Archive::new(decoder).unpack(&location)?;

    let extracted_dir = archive
        .as_ref()
        .file_name()
        .unwrap() // Safe here due to File::open check above
        .display()
        .to_string();
    let extracted_dir = extracted_dir
        .strip_suffix(".tar.bz2")
        .map(PathBuf::from)
        .ok_or(Error::InvalidArchiveFile(extracted_dir))?;
    let extracted_dir = location.as_ref().join(extracted_dir);

    let os_and_arch = OsAndArch::try_from(target)?;
    let OsAndArch { os, arch } = os_and_arch;
    let cef_dir = os_and_arch.to_string();
    let cef_dir = location.as_ref().join(cef_dir);

    if cef_dir.exists() {
        let old_dir = location.as_ref().join(format!("old_{os}_{arch}"));
        if show_progress {
            println!("Cleaning up: {}", old_dir.display());
        }
        fs_err::rename(&cef_dir, &old_dir)?;
        fs_err::remove_dir_all(old_dir)?;
    }
    const RELEASE_DIR: &str = "Release";
    fs_err::rename(extracted_dir.join(RELEASE_DIR), &cef_dir)?;

    if os != "macos" {
        let resources = extracted_dir.join("Resources");

        for entry in fs_err::read_dir(&resources)? {
            let entry = entry?;
            fs_err::rename(entry.path(), cef_dir.join(entry.file_name()))?;
        }
    }

    const CMAKE_LISTS_TXT: &str = "CMakeLists.txt";
    fs_err::rename(
        extracted_dir.join(CMAKE_LISTS_TXT),
        cef_dir.join(CMAKE_LISTS_TXT),
    )?;
    const CMAKE_DIR: &str = "cmake";
    fs_err::rename(extracted_dir.join(CMAKE_DIR), cef_dir.join(CMAKE_DIR))?;
    const INCLUDE_DIR: &str = "include";
    fs_err::rename(extracted_dir.join(INCLUDE_DIR), cef_dir.join(INCLUDE_DIR))?;
    const LIBCEF_DLL_DIR: &str = "libcef_dll";
    fs_err::rename(
        extracted_dir.join(LIBCEF_DLL_DIR),
        cef_dir.join(LIBCEF_DLL_DIR),
    )?;
    const CREDITS_HTML: &str = "CREDITS.html";
    fs_err::rename(extracted_dir.join(CREDITS_HTML), cef_dir.join(CREDITS_HTML))?;

    if show_progress {
        println!("Moved contents to: {}", cef_dir.display());
    }

    // Cleanup whatever is left in the extracted directory.
    let old_dir = extracted_dir
        .parent()
        .map(|parent| parent.join(format!("extracted_{os}_{arch}")))
        .ok_or_else(|| Error::InvalidArchiveFile(extracted_dir.display().to_string()))?;
    if show_progress {
        println!("Cleaning up: {}", old_dir.display());
    }
    fs_err::rename(&extracted_dir, &old_dir)?;
    fs_err::remove_dir_all(old_dir)?;

    Ok(cef_dir)
}

fn calculate_file_sha1(path: &Path) -> String {
    use std::io::Read;
    let mut file = BufReader::new(File::open(path).unwrap());
    let mut sha1 = Sha1::new();
    let mut buffer = [0; 8192];

    loop {
        let count = file.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        sha1.update(&buffer[..count]);
    }

    sha1.digest().to_string()
}

pub struct OsAndArch {
    pub os: &'static str,
    pub arch: &'static str,
}

impl Display for OsAndArch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let os = self.os;
        let arch = self.arch;
        write!(f, "cef_{os}_{arch}")
    }
}

impl TryFrom<&str> for OsAndArch {
    type Error = Error;

    fn try_from(target: &str) -> Result<Self> {
        match target {
            "aarch64-apple-darwin" => Ok(OsAndArch {
                os: "macos",
                arch: "aarch64",
            }),
            "x86_64-apple-darwin" => Ok(OsAndArch {
                os: "macos",
                arch: "x86_64",
            }),
            "x86_64-pc-windows-msvc" => Ok(OsAndArch {
                os: "windows",
                arch: "x86_64",
            }),
            "aarch64-pc-windows-msvc" => Ok(OsAndArch {
                os: "windows",
                arch: "aarch64",
            }),
            "i686-pc-windows-msvc" => Ok(OsAndArch {
                os: "windows",
                arch: "x86",
            }),
            "x86_64-unknown-linux-gnu" => Ok(OsAndArch {
                os: "linux",
                arch: "x86_64",
            }),
            "aarch64-unknown-linux-gnu" => Ok(OsAndArch {
                os: "linux",
                arch: "aarch64",
            }),
            "arm-unknown-linux-gnueabi" => Ok(OsAndArch {
                os: "linux",
                arch: "arm",
            }),
            v => Err(Error::UnsupportedTarget(v.to_string())),
        }
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub const DEFAULT_TARGET: &str = "x86_64-unknown-linux-gnu";
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
pub const DEFAULT_TARGET: &str = "aarch64-unknown-linux-gnu";
#[cfg(all(target_os = "linux", target_arch = "arm"))]
pub const DEFAULT_TARGET: &str = "arm-unknown-linux-gnueabi";

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
pub const DEFAULT_TARGET: &str = "x86_64-pc-windows-msvc";
#[cfg(all(target_os = "windows", target_arch = "x86"))]
pub const DEFAULT_TARGET: &str = "i686-pc-windows-msvc";
#[cfg(all(target_os = "windows", target_arch = "aarch64"))]
pub const DEFAULT_TARGET: &str = "aarch64-pc-windows-msvc";

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
pub const DEFAULT_TARGET: &str = "x86_64-apple-darwin";
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub const DEFAULT_TARGET: &str = "aarch64-apple-darwin";

#[cfg(test)]
mod lume_obs_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = env::temp_dir().join(format!(
                "lume-cef-sdk-contract-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(path.join("include")).unwrap();
            std::fs::write(path.join("include/cef_api_versions.h"), format!(
                "#define CEF_API_VERSION_15000 15000\n#if defined(OS_WIN)\n#define CEF_API_HASH_15000 \"windows\"\n#elif defined(OS_MAC)\n#define CEF_API_HASH_15000 \"{LUME_OBS_MAC_STABLE_HASH}\"\n#endif\n#define CEF_API_VERSION_999999 999999\n#if defined(OS_WIN)\n#define CEF_API_HASH_999999 \"windows\"\n#elif defined(OS_MAC)\n#define CEF_API_HASH_999999 \"{LUME_OBS_MAC_EXPERIMENTAL_HASH}\"\n#endif\n#define CEF_API_VERSION_LAST CEF_API_VERSION_15000\n"
            )).unwrap();
            std::fs::write(
                path.join("include/cef_version.h"),
                "#define CEF_VERSION \"150.0.19+gcf60f42+chromium-150.0.7871.252\"\n",
            )
            .unwrap();
            Self(path)
        }
        fn allows(&self, enabled: bool, mac_arm64: bool, expected: u64, archive: u64) -> bool {
            allow_lume_obs_sdk(
                &Version::new(150, 0, expected),
                &Version::new(150, 0, archive),
                &self.0,
                enabled,
                mac_arm64,
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn exact_obs_patch_requires_opt_in_target_and_both_version_numbers() {
        let fixture = Fixture::new();
        assert!(fixture.allows(true, true, 14, 19));
        assert!(!fixture.allows(false, true, 14, 19));
        assert!(!fixture.allows(true, false, 14, 19));
        assert!(!fixture.allows(true, true, 13, 19));
        assert!(!fixture.allows(true, true, 14, 20));
    }

    #[test]
    fn wrong_stable_or_experimental_api_hash_or_api_version_is_rejected() {
        for (old, replacement) in [
            (LUME_OBS_MAC_STABLE_HASH, "wrong-stable"),
            (LUME_OBS_MAC_EXPERIMENTAL_HASH, "wrong-experimental"),
            (
                "#define CEF_API_VERSION_LAST CEF_API_VERSION_15000",
                "#define CEF_API_VERSION_LAST CEF_API_VERSION_15600",
            ),
        ] {
            let fixture = Fixture::new();
            let path = fixture.0.join("include/cef_api_versions.h");
            let header = std::fs::read_to_string(&path).unwrap();
            std::fs::write(path, header.replace(old, replacement)).unwrap();
            assert!(!fixture.allows(true, true, 14, 19));
        }
    }

    #[test]
    fn archive_name_cannot_override_the_actual_sdk_header() {
        let fixture = Fixture::new();
        std::fs::write(
            fixture.0.join("include/cef_version.h"),
            "#define CEF_VERSION \"156.0.15+other\"\n",
        )
        .unwrap();
        assert!(!fixture.allows(true, true, 14, 19));
    }

    #[test]
    fn all_native_pins_require_their_own_platform_hashes_and_truthful_version() {
        let pins: serde_json::Value =
            serde_json::from_str(include_str!("../lume-sdk-pins.json")).unwrap();
        for pin in pins["targets"].as_object().unwrap().values() {
            let fixture = Fixture::new();
            let platform = pin["api_platform"].as_str().unwrap();
            let version = pin["cef_version"].as_str().unwrap();
            let api = format!(
                "#define CEF_API_VERSION_15000 15000\n#if defined(OS_{platform})\n#define CEF_API_HASH_15000 \"{}\"\n#endif\n#define CEF_API_VERSION_999999 999999\n#if defined(OS_{platform})\n#define CEF_API_HASH_999999 \"{}\"\n#endif\n#define CEF_API_VERSION_LAST CEF_API_VERSION_15000\n",
                pin["stable_hash"].as_str().unwrap(), pin["experimental_hash"].as_str().unwrap(),
            );
            std::fs::write(fixture.0.join("include/cef_api_versions.h"), api).unwrap();
            std::fs::write(
                fixture.0.join("include/cef_version.h"),
                format!("#define CEF_VERSION \"{version}\"\n"),
            )
            .unwrap();
            let name = format!(
                "cef_binary_{version}_{}.tar.xz",
                pin["cef_platform"].as_str().unwrap()
            );
            let target = pin["target"].as_str().unwrap();
            assert!(allow_lume_pinned_sdk(
                &Version::new(150, 0, 14),
                &name,
                &fixture.0,
                target
            ));
            assert!(!allow_lume_pinned_sdk(
                &Version::new(150, 0, 15),
                &name,
                &fixture.0,
                target
            ));
            assert!(!allow_lume_pinned_sdk(
                &Version::new(150, 0, 14),
                &name,
                &fixture.0,
                "unsupported-target"
            ));
            std::fs::write(
                fixture.0.join("include/cef_version.h"),
                "#define CEF_VERSION \"150.0.14\"\n",
            )
            .unwrap();
            assert!(!allow_lume_pinned_sdk(
                &Version::new(150, 0, 14),
                &name,
                &fixture.0,
                target
            ));
        }
    }

    #[test]
    fn pinned_builds_never_fall_back_to_another_sdk_download() {
        let previous = env::var_os("LUME_CEF_USE_PINNED_OBS");
        env::set_var("LUME_CEF_USE_PINNED_OBS", "1");
        let result = CefIndex::download_from("not-a-url");
        match previous {
            Some(value) => env::set_var("LUME_CEF_USE_PINNED_OBS", value),
            None => env::remove_var("LUME_CEF_USE_PINNED_OBS"),
        }
        assert!(matches!(result, Err(Error::PinnedSdkRequired)));
    }
}
