#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Acquire the exact OBS SDK for a native desktop build. No CDM is acquired.

Archive hashes come from OBS 33.0.0; API hashes were compared with the official
CEF 150.0.14 header expected by cef-rs. Pins are shared with download-cef's
strict, opt-in compatibility check. No version header or framework is patched.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import struct
import tarfile
import tempfile
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]
PIN_FILE = ROOT / "vendor/download-cef/lume-sdk-pins.json"
BASE_URL = "https://cdn-fastly.obsproject.com/downloads/"
MAX_ARCHIVE_BYTES = 1024 * 1024 * 1024
MAX_EXPANDED_BYTES = 4 * 1024 * 1024 * 1024
MARKER = ".lume-sdk.json"


def pins() -> dict:
    return json.loads(PIN_FILE.read_text(encoding="utf-8"))["targets"]


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def api_hash(text: str, api: str, platform: str) -> str:
    section = text.split(f"#define CEF_API_VERSION_{api} {api}", 1)[1]
    section = section.split("#define CEF_API_VERSION_", 1)[0]
    section = section.split(f"defined(OS_{platform})", 1)[1]
    return re.search(rf'#define\s+CEF_API_HASH_{api}\s+"([0-9a-f]{{40}})"', section)[1]


def validate_headers(sdk: Path, pin: dict) -> None:
    version = (sdk / "include/cef_version.h").read_text(encoding="utf-8")
    api = (sdk / "include/cef_api_versions.h").read_text(encoding="utf-8")
    if not re.search(r'^#define\s+CEF_VERSION\s+"' + re.escape(pin["cef_version"]) + r'"\s*$', version, re.M):
        raise ValueError("SDK version differs from the pinned native version")
    try:
        compatible = (
            "#define CEF_API_VERSION_LAST CEF_API_VERSION_15000" in api
            and api_hash(api, "15000", pin["api_platform"]) == pin["stable_hash"]
            and api_hash(api, "999999", pin["api_platform"]) == pin["experimental_hash"]
        )
    except (IndexError, TypeError):
        compatible = False
    if not compatible:
        raise ValueError("Stable or Experimental CEF API hash mismatch")


def runtime_library(sdk: Path, pin: dict) -> Path:
    if pin["os"] == "macos":
        return sdk / "Release/Chromium Embedded Framework.framework/Chromium Embedded Framework"
    return sdk / "Release" / ("libcef.dll" if pin["os"] == "windows" else "libcef.so")


def validate_binary(path: Path, pin: dict) -> None:
    with path.open("rb") as stream:
        header = stream.read(64)
        if pin["os"] == "macos":
            expected = 0x0100000C if pin["arch"] == "aarch64" else 0x01000007
            valid = header[:4] == b"\xcf\xfa\xed\xfe" and struct.unpack_from("<I", header, 4)[0] == expected
        elif pin["os"] == "linux":
            valid = header[:6] == b"\x7fELF\x02\x01" and struct.unpack_from("<H", header, 18)[0] == 62
        else:
            valid = header[:2] == b"MZ"
            if valid:
                stream.seek(struct.unpack_from("<I", header, 60)[0])
                pe = stream.read(6)
                valid = pe[:4] == b"PE\0\0" and struct.unpack_from("<H", pe, 4)[0] == 0x8664
    if not valid:
        raise ValueError("Runtime binary architecture does not match the requested platform")


def safe_member(name: str) -> None:
    path = PurePosixPath(name.replace("\\", "/"))
    if path.is_absolute() or ".." in path.parts or re.match(r"^[A-Za-z]:", name):
        raise ValueError("Unsafe SDK archive path")


def extract_archive(archive: Path, destination: Path) -> None:
    if archive.suffix == ".zip":
        with zipfile.ZipFile(archive) as source:
            members = source.infolist()
            if len(members) > 50_000 or sum(m.file_size for m in members) > MAX_EXPANDED_BYTES:
                raise ValueError("SDK expansion limit exceeded")
            for member in members:
                safe_member(member.filename)
                if (member.external_attr >> 16) & 0o170000 == 0o120000:
                    raise ValueError("SDK zip symlink rejected")
            source.extractall(destination)
    else:
        with tarfile.open(archive, "r:xz") as source:
            members = source.getmembers()
            if len(members) > 50_000 or sum(m.size for m in members) > MAX_EXPANDED_BYTES:
                raise ValueError("SDK expansion limit exceeded")
            for member in members:
                safe_member(member.name)
            source.extractall(destination, members=members, filter="data")


def normalize(sdk: Path, pin: dict) -> None:
    if pin["os"] == "macos":
        (sdk / "Chromium Embedded Framework.framework").symlink_to(
            "Release/Chromium Embedded Framework.framework", target_is_directory=True,
        )
        return
    # cef-dll-sys expects runtime files and locales at the SDK root. Keep the
    # upstream Release/Resources layout intact for CMake and source provenance.
    for folder in (sdk / "Release", sdk / "Resources"):
        if not folder.is_dir():
            continue
        for source in folder.iterdir():
            target = sdk / source.name
            if source.is_file():
                if target.exists():
                    if sha256(target) != sha256(source):
                        raise ValueError("Conflicting SDK root file")
                else:
                    try:
                        os.link(source, target)
                    except OSError:
                        shutil.copy2(source, target)
            elif source.name == "locales" and not target.exists():
                shutil.copytree(source, target)
    if not (sdk / "locales").is_dir():
        raise ValueError("SDK locale resources missing")
    if pin["os"] == "windows" and not (sdk / "bootstrap.exe").is_file():
        raise ValueError("SDK Windows sandbox bootstrap missing")


def prepare(platform: str, *, cache: Path | None = None, archive: Path | None = None,
            pin: dict | None = None) -> Path:
    pin = pin or pins()[platform]
    cache = (cache or ROOT / ".cache/cef-packaging").absolute()
    cache.mkdir(parents=True, exist_ok=True)
    if cache.is_symlink():
        raise ValueError("SDK cache must not be a symlink")
    archive = (archive or cache / pin["archive"]).absolute()
    if archive.is_symlink():
        raise ValueError("SDK archive must not be a symlink")
    if not archive.is_file():
        if archive.parent != cache:
            raise ValueError("An explicitly selected SDK archive must already exist")
        descriptor, temporary_name = tempfile.mkstemp(dir=cache)
        os.close(descriptor)
        temporary = Path(temporary_name)
        try:
            with temporary.open("wb") as outgoing:
                with urllib.request.urlopen(BASE_URL + pin["archive"], timeout=60) as incoming:
                    size = 0
                    while chunk := incoming.read(1024 * 1024):
                        size += len(chunk)
                        if size > MAX_ARCHIVE_BYTES:
                            raise ValueError("SDK download size limit exceeded")
                        outgoing.write(chunk)
            if sha256(temporary) != pin["sha256"]:
                raise ValueError("Official OBS SDK checksum mismatch")
            temporary.replace(archive)
        finally:
            temporary.unlink(missing_ok=True)
    if archive.stat().st_size > MAX_ARCHIVE_BYTES or sha256(archive) != pin["sha256"]:
        raise ValueError("Official OBS SDK checksum mismatch")
    parent = cache / platform
    parent.mkdir(exist_ok=True)
    if parent.is_symlink():
        raise ValueError("SDK platform directory must not be a symlink")
    sdk = parent / "sdk"
    if sdk.is_symlink():
        raise ValueError("SDK output must not be a symlink")
    if sdk.exists():
        marker = sdk / MARKER
        if not marker.is_file():
            raise ValueError("Refusing to replace an SDK not generated by this tool")
        metadata = json.loads(marker.read_text())
        if metadata["sha256"] != pin["sha256"] or metadata["platform"] != platform:
            raise ValueError("Existing SDK provenance differs from pins")
        if pin["os"] == "linux" and "Release/chrome-sandbox" not in metadata["critical_files"]:
            raise ValueError("Regenerate this SDK cache to record the sandbox helper provenance")
        validate_headers(sdk, pin)
        for name, expected in metadata["critical_files"].items():
            if sha256(sdk / name) != expected:
                raise ValueError("Prepared SDK file was modified")
        return sdk
    with tempfile.TemporaryDirectory(prefix=".extract-", dir=parent) as temporary:
        staging = Path(temporary)
        extract_archive(archive, staging)
        candidates = [staging] if (staging / "include/cef_version.h").is_file() else [
            path.parent.parent for path in staging.glob("*/include/cef_version.h")
        ]
        if len(candidates) != 1:
            raise ValueError("Unexpected SDK archive layout")
        extracted = candidates[0]
        validate_headers(extracted, pin)
        library = runtime_library(extracted, pin)
        validate_binary(library, pin)
        for name in ("CMakeLists.txt", "LICENSE.txt", "CREDITS.html"):
            if not (extracted / name).is_file():
                raise ValueError("SDK build files or notices missing")
        critical = ["include/cef_version.h", "include/cef_api_versions.h", "CMakeLists.txt",
                    library.relative_to(extracted).as_posix()]
        if pin["os"] == "windows":
            critical.append("Release/bootstrap.exe")
        if pin["os"] == "linux":
            critical.append("Release/chrome-sandbox")
        metadata = {"platform": platform, "sha256": pin["sha256"],
                    "source_url": BASE_URL + pin["archive"],
                    "critical_files": {name: sha256(extracted / name) for name in critical}}
        normalize(extracted, pin)
        archive_info = {"type": "standard", "sha1": hashlib.sha1(archive.read_bytes()).hexdigest(),
                        "name": f'cef_binary_{pin["cef_version"]}_{pin["cef_platform"]}.tar.xz',
                        "lume_obs_provenance": metadata}
        (extracted / "archive.json").write_text(json.dumps(archive_info, indent=2) + "\n")
        (extracted / MARKER).write_text(json.dumps(metadata, indent=2) + "\n")
        # Moving the extraction root itself would confuse TemporaryDirectory's
        # cleanup; move its contents into a fresh generated SDK directory.
        sdk.mkdir()
        for child in list(extracted.iterdir()):
            child.rename(sdk / child.name)
    return sdk


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", choices=pins(), required=True)
    parser.add_argument("--archive", type=Path)
    args = parser.parse_args()
    print(prepare(args.platform, archive=args.archive))
