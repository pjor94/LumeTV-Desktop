// SPDX-License-Identifier: GPL-3.0-or-later
use super::{
    install::{replace_with, safe_path},
    network::{allowed, copy_verified},
    policy::*,
    Error,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use ring::{
    rand::SystemRandom,
    signature::{Ed25519KeyPair, KeyPair},
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

fn manifest() -> Manifest {
    let targets = ["macos-arm64", "macos-x64", "windows-x64", "linux-x64"]
        .into_iter()
        .map(|platform| {
            (
                platform.to_owned(),
                Target {
                    archive: Artifact {
                        name: format!(
                            "LumeTV-1.2.3-{platform}.{}",
                            if platform == "linux-x64" {
                                "tar.gz"
                            } else {
                                "zip"
                            }
                        ),
                        size: 3,
                        sha256: format!("{:x}", Sha256::digest(b"abc")),
                    },
                    installer: None,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    Manifest {
        schema: 1,
        application_id: "it.lumetv.desktop".into(),
        repository: REPOSITORY.into(),
        version: "1.2.3".into(),
        prerelease: false,
        notes: "Note di prova".into(),
        targets,
    }
}
#[test]
fn verifies_ed25519_before_accepting_metadata() {
    let key = Ed25519KeyPair::from_pkcs8(
        Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
            .unwrap()
            .as_ref(),
    )
    .unwrap();
    let json = serde_json::to_string(&manifest()).unwrap();
    let mut signed = SignedManifest {
        signature: STANDARD.encode(key.sign(json.as_bytes()).as_ref()),
        json,
    };
    assert_eq!(
        signed
            .verify_with_key(key.public_key().as_ref())
            .unwrap()
            .version,
        "1.2.3"
    );
    signed.json.push(' ');
    assert!(matches!(
        signed.verify_with_key(key.public_key().as_ref()),
        Err(Error::Signature)
    ));
    assert!(matches!(signed.verify(), Err(Error::Signature)));
}
#[test]
fn denies_wrong_application_repository_or_package_identity() {
    let mut item = manifest();
    item.application_id = "another.app".into();
    assert_eq!(item.validate(), Err(Error::Metadata));
    let mut item = manifest();
    item.repository = "someone/fork".into();
    assert_eq!(item.validate(), Err(Error::Metadata));
    let mut item = manifest();
    item.targets.get_mut("windows-x64").unwrap().archive.name = "lume.exe".into();
    assert_eq!(item.validate(), Err(Error::Metadata));
    let mut item = manifest();
    item.targets.get_mut("linux-x64").unwrap().archive.size = MAX_PACKAGE + 1;
    assert_eq!(item.validate(), Err(Error::Metadata));
}
#[test]
fn compares_versions_numerically_and_never_downgrades() {
    let mut item = manifest();
    assert!(item.eligible("1.2.2", false).unwrap());
    assert!(!item.eligible("1.2.3", true).unwrap());
    assert!(!item.eligible("1.10.0", true).unwrap());
    item.prerelease = true;
    assert!(!item.eligible("0.1.0", false).unwrap());
    assert!(item.eligible("0.1.0", true).unwrap());
    for invalid in ["../1.2.3", "1.2.3+url", "v1.2.3", "1.2"] {
        assert!(version(invalid).is_err());
    }
}
#[test]
fn limits_feed_and_skips_legacy_draft_or_unwanted_preview() {
    let make = |v: &str, draft: bool, prerelease: bool| serde_json::json!({"tag_name":v,"draft":draft,"prerelease":prerelease,"assets":[{"name":"update.json","size":100,"state":"uploaded","browser_download_url":"ignored"},{"name":"update.sig","size":89,"state":"uploaded","browser_download_url":"ignored"}]});
    let raw = serde_json::to_vec(&vec![
        make("v1.2.3", false, false),
        make("v1.10.0", false, false),
        make("v2.0.0", true, false),
        make("v3.0.0", false, true),
    ])
    .unwrap();
    assert_eq!(
        releases(&raw, "0.1.0", false)
            .unwrap()
            .iter()
            .map(|r| r.tag_name.as_str())
            .collect::<Vec<_>>(),
        ["v1.10.0", "v1.2.3"]
    );
    assert_eq!(releases(&raw, "0.1.0", true).unwrap().len(), 3);
    assert!(releases(&vec![b' '; 2 * 1024 * 1024 + 1], "0.1.0", true).is_err());
}
#[test]
fn only_accepts_canonical_release_assets() {
    let item = manifest();
    let artifact = &item.targets["windows-x64"].archive;
    let mut release = Release {
        tag_name: "v1.2.3".into(),
        draft: false,
        prerelease: false,
        assets: vec![ReleaseAsset {
            name: artifact.name.clone(),
            size: 3,
            state: "uploaded".into(),
            browser_download_url: asset_url("1.2.3", &artifact.name).unwrap(),
            digest: Some(format!("sha256:{}", artifact.sha256)),
        }],
    };
    assert!(release.cross_check(&item, artifact).is_ok());
    release.assets[0]
        .browser_download_url
        .push_str("?redirect=evil");
    assert!(release.cross_check(&item, artifact).is_err());
    assert!(asset_url("1.2.3", "../payload.zip").is_err());
}
#[test]
fn refuses_transport_downgrades_credential_urls_and_other_hosts() {
    for url in [
        "http://github.com/file",
        "https://github.com.evil.test/file",
        "https://user@github.com/file",
        "https://github.com:444/file",
        "https://localhost/file",
        "https://github.com/file#fragment",
    ] {
        assert!(!allowed(&reqwest::Url::parse(url).unwrap()), "{url}");
    }
    assert!(allowed(
        &reqwest::Url::parse("https://release-assets.githubusercontent.com/file?signature=example")
            .unwrap()
    ));
}
#[test]
fn streamed_download_checks_size_hash_and_cancellation() {
    let artifact = manifest().targets["windows-x64"].archive.clone();
    let mut progress = 0;
    let counter = std::cell::Cell::new(0);
    assert!(copy_verified(
        &mut &b"abc"[..],
        &mut vec![],
        &artifact,
        &AtomicBool::new(false),
        |n| counter.set(n)
    )
    .is_ok());
    progress += counter.get();
    assert_eq!(progress, 3);
    for input in [b"ab".as_slice(), b"abcd".as_slice(), b"xyz".as_slice()] {
        assert_eq!(
            copy_verified(
                &mut &input[..],
                &mut vec![],
                &artifact,
                &AtomicBool::new(false),
                |_| {}
            ),
            Err(Error::Checksum)
        );
    }
    assert_eq!(
        copy_verified(
            &mut &b"abc"[..],
            &mut vec![],
            &artifact,
            &AtomicBool::new(true),
            |_| {}
        ),
        Err(Error::Cancelled)
    );
    let cancel = AtomicBool::new(false);
    assert_eq!(
        copy_verified(&mut &b"abc"[..], &mut vec![], &artifact, &cancel, |_| {
            cancel.store(true, Ordering::Relaxed)
        }),
        Err(Error::Cancelled)
    );
}
#[test]
fn rejects_archive_traversal_on_all_platforms() {
    for path in [
        "../outside",
        "/tmp/file",
        "C:/file",
        "folder\\file",
        "app/../file",
        "app/./file",
        "app/file:stream",
        "",
    ] {
        assert!(safe_path(path).is_err(), "{path}");
    }
    assert_eq!(
        safe_path("Lume TV.app/Contents/MacOS/lume-cef").unwrap(),
        PathBuf::from("Lume TV.app/Contents/MacOS/lume-cef")
    );
}
#[test]
fn rolls_back_if_the_new_package_cannot_be_moved() {
    let root = std::env::temp_dir().join(format!("lume-rollback-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    let old = root.join("app");
    let new = root.join("new");
    let backup = root.join("backup");
    fs::create_dir(&old).unwrap();
    fs::create_dir(&new).unwrap();
    fs::write(old.join("version"), b"old").unwrap();
    let mut calls = 0;
    assert_eq!(
        replace_with(&old, &new, &backup, |a, b| {
            calls += 1;
            if calls == 2 {
                Err(std::io::Error::other("injected move failure"))
            } else {
                fs::rename(a, b)
            }
        }),
        Err(Error::Install)
    );
    assert_eq!(fs::read(old.join("version")).unwrap(), b"old");
    assert!(!backup.exists());
    assert!(new.exists());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn successful_swap_keeps_the_previous_package() {
    let root = std::env::temp_dir().join(format!("lume-swap-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    let old = root.join("app");
    let new = root.join("new");
    let backup = root.join("backup");
    fs::create_dir(&old).unwrap();
    fs::create_dir(&new).unwrap();
    fs::write(old.join("version"), b"old").unwrap();
    fs::write(new.join("version"), b"new").unwrap();
    replace_with(&old, &new, &backup, |a, b| fs::rename(a, b)).unwrap();
    assert_eq!(fs::read(old.join("version")).unwrap(), b"new");
    assert_eq!(fs::read(backup.join("version")).unwrap(), b"old");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn preferences_and_server_backoff_survive_restart_without_network_requests() {
    assert!(!super::service::Settings::default().preview);
    let root = std::env::temp_dir().join(format!("lume-update-settings-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(root.join("updates")).unwrap();
    let deadline = super::service::now() + 600;
    fs::write(root.join("updates/settings.json"), serde_json::to_vec(&serde_json::json!({"settings":{"automatic":false,"preview":false},"retry_after":deadline})).unwrap()).unwrap();
    let service = super::UpdateService::new(&root);
    assert!(!service.snapshot().settings.automatic);
    assert!(!service.snapshot().settings.preview);
    assert_eq!(service.check(false).phase, "idle");
    let snapshot = service.check(true);
    assert_eq!(snapshot.error, Some(Error::RateLimited));
    assert_eq!(snapshot.next_check_at, deadline);
    service.settings(None, Some(true)).unwrap();
    assert_eq!(service.check(true).phase, "idle");
    assert_eq!(service.snapshot().next_check_at, deadline);
    drop(service);
    let service = super::UpdateService::new(&root);
    assert!(service.snapshot().settings.preview);
    assert!(!service.snapshot().settings.automatic);
    assert_eq!(service.check(true).error, Some(Error::RateLimited));
    drop(service);
    fs::remove_dir_all(root).unwrap();
}
