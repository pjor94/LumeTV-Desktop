# Local compatibility exception

Upstream: `download-cef` 2.3.2, from `tauri-apps/cef-rs` commit
`0c577ce44dbd36952ac3721b577c6e423ceff44f`, package path `download-cef`.
Original license: Apache-2.0 OR MIT. All upstream implementation remains intact
except the additional opt-in branch in `check_archive_json` and its helpers.

`LUME_CEF_ALLOW_OBS_150_0_19=1` permits exactly the macOS ARM64 build host,
bindings expecting CEF 150.0.14, and the OBS CEF 150.0.19 SDK. It checks the actual
SDK version header, Stable API 15000 hash and Experimental API 999999 hash.
Without the separately pinned opt-in below, every other newer-version
combination still fails the upstream version check.
`archive.json` retains the actual OBS 150.0.19 version.

Original CEF 150.0.14 is commit `7c1aa68455db1f1fad159c2b83070ad318212b3d`.
Its tracked Stable 15000 macOS hash is
`e85c868aa8a947e1e7c9904c0745c60ae490fa05`:
https://github.com/chromiumembedded/cef/blob/7c1aa68/cef_api_versions.json

The generated original SDK header was extracted from the official archive
`cef_binary_150.0.14+g7c1aa68+chromium-150.0.7871.129_macosarm64_minimal.tar.bz2`.
Its Experimental macOS hash is
`0934859000c911cdd75907f63f2ef8e485706e5b`, identical to OBS 150.0.19. Both the
normalizer and the exception check this value. `cef-dll-sys` 150.2.1 ships
Experimental bindings: the Rust client and its C++ wrapper must consistently
select API 999999. Those Experimental bindings require a matching Experimental
hash as well as Stable API compatibility.

CEF explains API versioning and the need to keep wrapper and client API
configuration aligned here:
https://github.com/chromiumembedded/cef/blob/master/docs/api_versioning.md

The OBS archive and its SHA256 are identified by OBS Studio 33.0.0:
https://github.com/obsproject/obs-studio/blob/33.0.0/CMakePresets.json

The patch leaves the Chromium framework, CDM, codec implementation, signatures
and VMP behavior unchanged. Media playback requires a separate runtime test.

## Native package pins — 9 November 2026

`LUME_CEF_USE_PINNED_OBS=1` adds exactly the four native targets recorded in
[lume-sdk-pins.json](lume-sdk-pins.json). Python SDK preparation and the Rust
compatibility check read the same file. The old ARM64 flag remains supported.

Mac ARM64/Intel and Linux x64 use OBS CEF 150.0.19. Windows x64 reports
`150.0.20-7871-devtools_patch.3553+gc4b3522+chromium-150.0.7871.252`.
The Windows suffix is not SemVer: its exact pinned name is checked before the
upstream parser, without renaming it to another CEF version. All four retain
their actual SDK version in `archive.json` and packaged runtime metadata.

Archive SHA-256 values were checked against OBS 33.0.0. Stable API 15000 and
Experimental API 999999 hashes in every archive were compared with the original
official CEF 150.0.14 generated header, including WIN and LINUX branches.
Each pin requires its own target, actual version and both platform hashes.
Unknown targets, altered versions and mismatched hashes remain rejected.

Windows packages use the matching SDK's `bootstrap.exe` with the app DLL's
`RunWinMain` entry point, following
[cef-rs's sandbox example](https://github.com/tauri-apps/cef-rs/blob/0c577ce44dbd36952ac3721b577c6e423ceff44f/examples/cefsimple/src/win.rs).
The browser uses the sandbox context supplied by the bootstrap, with the sandbox enabled.
