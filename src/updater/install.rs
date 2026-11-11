// SPDX-License-Identifier: GPL-3.0-or-later
use super::{
    network::{private_file, verify_file},
    policy::{self, Artifact, SignedManifest},
    Error, Result,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstallMode {
    Replace,
    System,
    Open,
}

#[derive(Clone)]
pub struct Layout {
    pub root: PathBuf,
    pub helper: PathBuf,
    pub mode: InstallMode,
}
impl Layout {
    pub fn detect() -> Option<Self> {
        let executable = std::env::current_exe().ok()?.canonicalize().ok()?;
        let (root, helper) = if cfg!(target_os = "macos") {
            let root = executable
                .ancestors()
                .find(|p| p.extension().is_some_and(|e| e == "app"))?
                .to_path_buf();
            (root.clone(), root.join("Contents/MacOS/lume-updater"))
        } else {
            let expected = if cfg!(windows) {
                "LumeTV.exe"
            } else {
                "lume-cef"
            };
            if executable.file_name()? != expected {
                return None;
            }
            let root = executable.parent()?.to_path_buf();
            (
                root.clone(),
                root.join(if cfg!(windows) {
                    "lume-updater.exe"
                } else {
                    "lume-updater"
                }),
            )
        };
        let system = cfg!(target_os = "linux") && root == Path::new("/opt/lume-tv");
        let writable = root.parent().is_some_and(writable_directory);
        let mode = if system {
            InstallMode::System
        } else if helper.is_file() && writable {
            InstallMode::Replace
        } else {
            InstallMode::Open
        };
        Some(Self { root, helper, mode })
    }
}
fn writable_directory(path: &Path) -> bool {
    let probe = path.join(format!(".lume-write-{}", uuid::Uuid::new_v4()));
    let Ok(file) = OpenOptions::new().write(true).create_new(true).open(&probe) else {
        return false;
    };
    drop(file);
    let _ = fs::remove_file(probe);
    true
}

#[derive(Serialize, Deserialize)]
pub struct Plan {
    pub signed: SignedManifest,
    pub platform: String,
    pub archive: PathBuf,
    pub destination: PathBuf,
    pub parent: u32,
}
pub fn spawn_helper(
    layout: &Layout,
    signed: &SignedManifest,
    archive: &Path,
    directory: &Path,
) -> Result<()> {
    let folder = directory.join(format!("install-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&folder).map_err(|_| Error::Disk)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o700)).map_err(|_| Error::Disk)?;
    }
    let helper = folder.join(if cfg!(windows) {
        "lume-updater.exe"
    } else {
        "lume-updater"
    });
    fs::copy(&layout.helper, &helper).map_err(|_| Error::Disk)?;
    let plan = Plan {
        signed: signed.clone(),
        platform: policy::platform().ok_or(Error::Unsupported)?.to_string(),
        archive: archive.to_path_buf(),
        destination: layout.root.clone(),
        parent: std::process::id(),
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(folder.join("plan.json"))
        .map_err(|_| Error::Disk)?;
    private_file(&file)?;
    file.write_all(&serde_json::to_vec(&plan).map_err(|_| Error::Metadata)?)
        .map_err(|_| Error::Disk)?;
    file.sync_all().map_err(|_| Error::Disk)?;
    let mut child = Command::new(helper)
        .arg("--apply")
        .arg(folder.join("plan.json"))
        .current_dir(&folder)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| Error::Install)?;
    // Close the player only after the independent helper has verified its plan.
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(45) {
        if folder.join("prepared").is_file() {
            return Ok(());
        }
        if child.try_wait().map_err(|_| Error::Install)?.is_some() {
            return Err(Error::Install);
        }
        thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    Err(Error::Install)
}
pub fn open_package(path: &Path) -> Result<()> {
    let command = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer.exe"
    } else {
        "xdg-open"
    };
    let mut child = Command::new(command)
        .arg(path)
        .spawn()
        .map_err(|_| Error::Install)?;
    // Surface a missing system handler instead of reporting an opened installer.
    for _ in 0..20 {
        if let Some(status) = child.try_wait().map_err(|_| Error::Install)? {
            return if status.success() {
                Ok(())
            } else {
                Err(Error::Install)
            };
        }
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

pub fn run_helper(plan_path: &Path) -> Result<()> {
    let bytes = fs::read(plan_path).map_err(|_| Error::Disk)?;
    if bytes.len() > 128 * 1024 {
        return Err(Error::Metadata);
    }
    let plan: Plan = serde_json::from_slice(&bytes).map_err(|_| Error::Metadata)?;
    let manifest = plan.signed.verify()?;
    if Some(plan.platform.as_str()) != policy::platform()
        || !manifest.eligible(env!("CARGO_PKG_VERSION"), true)?
    {
        return Err(Error::Metadata);
    }
    let artifact = &manifest
        .targets
        .get(&plan.platform)
        .ok_or(Error::Metadata)?
        .archive;
    verify_file(&plan.archive, artifact)?;
    if !plan.destination.is_absolute()
        || plan.destination.canonicalize().ok().as_ref() != Some(&plan.destination)
        || !plan.destination.is_dir()
    {
        return Err(Error::Install);
    }
    // Validate the installed layout before accepting any replacement destination.
    validate_payload(&plan.destination, env!("CARGO_PKG_VERSION"), &plan.platform)?;
    fs::write(
        plan_path.parent().ok_or(Error::Metadata)?.join("prepared"),
        b"ready",
    )
    .map_err(|_| Error::Disk)?;
    wait_parent(plan.parent)?;
    let result = apply(&plan, artifact, &manifest.version);
    if let Some(directory) = plan_path.parent().and_then(Path::parent) {
        let value = serde_json::json!({"version":manifest.version,"success":result.is_ok(),"error":result.as_ref().err()});
        let _ = fs::write(
            directory.join("install-result.json"),
            serde_json::to_vec(&value).unwrap_or_default(),
        );
    }
    // A failed swap restores the previous package; reopen it so the error is visible.
    if plan.destination.is_dir() {
        let _ = relaunch(&plan.destination);
    }
    result
}
fn wait_parent(pid: u32) -> Result<()> {
    if pid == 0 || pid == std::process::id() {
        return Err(Error::Install);
    }
    let start = Instant::now();
    while process_exists(pid) {
        if start.elapsed() > Duration::from_secs(60) {
            return Err(Error::Install);
        }
        thread::sleep(Duration::from_millis(250));
    }
    Ok(())
}
#[cfg(unix)]
fn process_exists(pid: u32) -> bool {
    unsafe {
        libc::kill(pid as i32, 0) == 0
            || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }
}
#[cfg(windows)]
fn process_exists(pid: u32) -> bool {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_TIMEOUT},
        System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
    };
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            return false;
        }
        let active = WaitForSingleObject(handle, 0) == WAIT_TIMEOUT;
        CloseHandle(handle);
        active
    }
}
fn apply(plan: &Plan, artifact: &Artifact, version: &str) -> Result<()> {
    let parent = plan.destination.parent().ok_or(Error::Install)?;
    let stage = parent.join(format!(".lume-update-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&stage).map_err(|_| Error::Disk)?;
    let result = (|| {
        unpack(&plan.archive, artifact, &stage)?;
        let name = if plan.platform.starts_with("macos-") {
            "Lume TV.app".to_owned()
        } else {
            format!("LumeTV-{version}-{}", plan.platform)
        };
        let payload = stage.join(name);
        validate_payload(&payload, version, &plan.platform)?;
        let backup = parent.join(format!(
            ".lume-backup-{}-{}",
            env!("CARGO_PKG_VERSION"),
            uuid::Uuid::new_v4()
        ));
        replace_with(&plan.destination, &payload, &backup, |from, to| {
            fs::rename(from, to)
        })
    })();
    let _ = fs::remove_dir_all(&stage);
    result
}
pub(crate) fn replace_with(
    destination: &Path,
    payload: &Path,
    backup: &Path,
    mut rename: impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> Result<()> {
    rename(destination, backup).map_err(|_| Error::Install)?;
    if rename(payload, destination).is_err() {
        rename(backup, destination).map_err(|_| Error::Install)?;
        return Err(Error::Install);
    }
    Ok(())
}

const MAX_EXPANDED: u64 = 6 * 1024 * 1024 * 1024;
pub(crate) fn safe_path(value: &str) -> Result<PathBuf> {
    if value.is_empty()
        || value.contains(['\\', ':', '\0'])
        || value.starts_with('/')
        || value.split('/').any(|c| c == "." || c == "..")
    {
        return Err(Error::Metadata);
    }
    let path = PathBuf::from(value);
    if path
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(Error::Metadata);
    }
    Ok(path)
}
fn unpack(archive: &Path, artifact: &Artifact, stage: &Path) -> Result<()> {
    if artifact.name.ends_with(".tar.gz") {
        let gzip = flate2::read::GzDecoder::new(fs::File::open(archive).map_err(|_| Error::Disk)?);
        let mut tar = tar::Archive::new(gzip);
        let mut size = 0u64;
        let mut seen = HashSet::new();
        for (index, entry) in tar.entries().map_err(|_| Error::Metadata)?.enumerate() {
            if index > 100_000 {
                return Err(Error::Metadata);
            }
            let mut entry = entry.map_err(|_| Error::Metadata)?;
            let name = entry
                .path()
                .map_err(|_| Error::Metadata)?
                .to_str()
                .ok_or(Error::Metadata)?
                .to_owned();
            let relative = safe_path(&name)?;
            if !seen.insert(relative.clone()) {
                return Err(Error::Metadata);
            }
            size = size.checked_add(entry.size()).ok_or(Error::Metadata)?;
            let kind = entry.header().entry_type();
            if size > MAX_EXPANDED || (!kind.is_file() && !kind.is_dir()) {
                return Err(Error::Metadata);
            }
            let output = stage.join(relative);
            if kind.is_dir() {
                fs::create_dir_all(output).map_err(|_| Error::Disk)?;
            } else {
                fs::create_dir_all(output.parent().ok_or(Error::Metadata)?)
                    .map_err(|_| Error::Disk)?;
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&output)
                    .map_err(|_| Error::Disk)?;
                std::io::copy(&mut entry, &mut file).map_err(|_| Error::Disk)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(
                        output,
                        fs::Permissions::from_mode(entry.header().mode().unwrap_or(0o644) & 0o777),
                    )
                    .map_err(|_| Error::Disk)?;
                }
            }
        }
    } else {
        let mut zip = zip::ZipArchive::new(fs::File::open(archive).map_err(|_| Error::Disk)?)
            .map_err(|_| Error::Metadata)?;
        if zip.len() > 100_000 {
            return Err(Error::Metadata);
        }
        let mut names = HashSet::new();
        let mut links = HashSet::new();
        let mut size = 0u64;
        for index in 0..zip.len() {
            let mut entry = zip.by_index(index).map_err(|_| Error::Metadata)?;
            let path = safe_path(&entry.name().map_err(|_| Error::Metadata)?)?;
            if !names.insert(path.clone()) {
                return Err(Error::Metadata);
            }
            size = size.checked_add(entry.size()).ok_or(Error::Metadata)?;
            if size > MAX_EXPANDED {
                return Err(Error::Metadata);
            }
            if entry.is_symlink() {
                if !cfg!(target_os = "macos") || entry.size() > 4096 {
                    return Err(Error::Metadata);
                }
                let mut target = String::new();
                entry
                    .read_to_string(&mut target)
                    .map_err(|_| Error::Metadata)?;
                let target = Path::new(&target);
                if target.is_absolute()
                    || target
                        .components()
                        .any(|part| !matches!(part, Component::Normal(_)))
                {
                    return Err(Error::Metadata);
                }
                links.insert(path);
            } else if !entry.is_dir()
                && entry
                    .unix_mode()
                    .is_some_and(|mode| mode & 0o170000 != 0 && mode & 0o170000 != 0o100000)
            {
                return Err(Error::Metadata);
            }
        }
        // Never extract an entry through a previously created symlink.
        if names
            .iter()
            .any(|p| p.ancestors().skip(1).any(|a| links.contains(a)))
        {
            return Err(Error::Metadata);
        }
        if cfg!(target_os = "macos") {
            let status = Command::new("/usr/bin/ditto")
                .args(["-x", "-k"])
                .arg(archive)
                .arg(stage)
                .status()
                .map_err(|_| Error::Install)?;
            if !status.success() {
                return Err(Error::Install);
            }
        } else {
            for index in 0..zip.len() {
                let mut entry = zip.by_index(index).map_err(|_| Error::Metadata)?;
                let output = stage.join(safe_path(&entry.name().map_err(|_| Error::Metadata)?)?);
                if entry.is_dir() {
                    fs::create_dir_all(output).map_err(|_| Error::Disk)?;
                } else {
                    fs::create_dir_all(output.parent().ok_or(Error::Metadata)?)
                        .map_err(|_| Error::Disk)?;
                    let mut file = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(output)
                        .map_err(|_| Error::Disk)?;
                    std::io::copy(&mut entry, &mut file).map_err(|_| Error::Disk)?;
                }
            }
        }
    }
    Ok(())
}
fn validate_payload(root: &Path, version: &str, platform: &str) -> Result<()> {
    if platform.starts_with("macos-") {
        let plist = root.join("Contents/Info.plist");
        for (key, expected) in [
            ("CFBundleIdentifier", "it.lumetv.desktop"),
            ("CFBundleShortVersionString", version),
        ] {
            let output = Command::new("/usr/libexec/PlistBuddy")
                .arg("-c")
                .arg(format!("Print :{key}"))
                .arg(&plist)
                .output()
                .map_err(|_| Error::Install)?;
            if !output.status.success()
                || String::from_utf8_lossy(&output.stdout).trim() != expected
            {
                return Err(Error::Metadata);
            }
        }
        for path in [
            "Contents/MacOS/lume-cef",
            "Contents/MacOS/lume-updater",
            "Contents/Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework",
        ] {
            if !root.join(path).is_file() {
                return Err(Error::Metadata);
            }
        }
        if !Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(root)
            .status()
            .map_err(|_| Error::Install)?
            .success()
        {
            return Err(Error::Signature);
        }
    } else {
        let runtime: serde_json::Value = serde_json::from_slice(
            &fs::read(root.join("runtime.json")).map_err(|_| Error::Metadata)?,
        )
        .map_err(|_| Error::Metadata)?;
        if runtime["app_version"] != version
            || runtime["platform"] != platform
            || runtime["widevine_cdm_bundled"] != false
        {
            return Err(Error::Metadata);
        }
        let files = if platform == "windows-x64" {
            vec![
                "LumeTV.exe",
                "LumeTV.dll",
                "libcef.dll",
                "lume-updater.exe",
                "icudtl.dat",
                "resources.pak",
            ]
        } else {
            vec![
                "AppRun",
                "lume-cef",
                "libcef.so",
                "lume-updater",
                "icudtl.dat",
                "resources.pak",
                "chrome-sandbox",
            ]
        };
        if files.iter().any(|name| !root.join(name).is_file()) || !root.join("locales").is_dir() {
            return Err(Error::Metadata);
        }
    }
    Ok(())
}
fn relaunch(root: &Path) -> Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        let mut cmd = Command::new("open");
        cmd.arg("-n").arg(root);
        cmd
    } else {
        Command::new(root.join(if cfg!(windows) {
            "LumeTV.exe"
        } else {
            "AppRun"
        }))
    };
    command
        .current_dir(root.parent().ok_or(Error::Install)?)
        .spawn()
        .map_err(|_| Error::Install)?;
    Ok(())
}

#[cfg(test)]
mod archive_tests {
    use super::*;
    use sha2::{Digest, Sha256};
    fn temporary() -> PathBuf {
        let path = std::env::temp_dir().join(format!("lume-archive-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        path
    }
    fn artifact(path: &Path, extension: &str) -> Artifact {
        let bytes = fs::read(path).unwrap();
        Artifact {
            name: format!("test.{extension}"),
            size: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(bytes)),
        }
    }
    #[test]
    fn refuses_a_zip_symlink_escape_and_entries_below_a_symlink() {
        for (target, nested) in [("../../outside", false), ("safe", true)] {
            let root = temporary();
            let path = root.join("payload.zip");
            let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            zip.add_symlink("app/link", target, options).unwrap();
            if nested {
                zip.start_file("app/link/file", options).unwrap();
                zip.write_all(b"payload").unwrap();
            }
            zip.finish().unwrap();
            let stage = root.join("stage");
            fs::create_dir(&stage).unwrap();
            assert_eq!(
                unpack(&path, &artifact(&path, "zip"), &stage),
                Err(Error::Metadata)
            );
            assert!(!root.join("outside").exists());
            fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn extracts_a_regular_tar_with_modes_but_rejects_links() {
        for link in [false, true] {
            let root = temporary();
            let path = root.join("payload.tar.gz");
            let gzip = flate2::write::GzEncoder::new(
                fs::File::create(&path).unwrap(),
                flate2::Compression::default(),
            );
            let mut tar = tar::Builder::new(gzip);
            let mut header = tar::Header::new_gnu();
            header.set_mode(0o755);
            if link {
                header.set_size(0);
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_link_name("/tmp/outside").unwrap();
            } else {
                header.set_size(3);
            }
            header.set_cksum();
            tar.append_data(
                &mut header,
                "app/AppRun",
                if link {
                    b"".as_slice()
                } else {
                    b"abc".as_slice()
                },
            )
            .unwrap();
            tar.into_inner().unwrap().finish().unwrap();
            let stage = root.join("stage");
            fs::create_dir(&stage).unwrap();
            let result = unpack(&path, &artifact(&path, "tar.gz"), &stage);
            if link {
                assert_eq!(result, Err(Error::Metadata));
            } else {
                result.unwrap();
                assert_eq!(fs::read(stage.join("app/AppRun")).unwrap(), b"abc");
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    assert_eq!(
                        fs::metadata(stage.join("app/AppRun"))
                            .unwrap()
                            .permissions()
                            .mode()
                            & 0o7777,
                        0o755
                    );
                }
            }
            fs::remove_dir_all(root).unwrap();
        }
    }
}
