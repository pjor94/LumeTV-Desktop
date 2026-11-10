#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Validate and normalize an already extracted official OBS macOS ARM64 CEF SDK.

The original OBS archive is checked against the published OBS 33.0.0 SHA256.
The SDK's two API headers, CMake entrypoint and framework executable must match
that verified archive byte for byte. No framework binaries or headers change.
Normalization adds a relative root framework symlink and truthful archive.json
for cef-rs's SDK discovery. This avoids duplicating the large framework.

The SDK must be below this project's .cache/cef-obs directory. This helper never
modifies an installed application, signs a binary, downloads a component, or
enables codecs/DRM. Run --check for a read-only validation.

Sources:
https://github.com/obsproject/obs-studio/blob/33.0.0/CMakePresets.json
https://github.com/obsproject/obs-studio/blob/33.0.0/cmake/macos/buildspec.cmake
https://github.com/chromiumembedded/cef/blob/7c1aa68/cef_api_versions.json
https://github.com/chromiumembedded/cef/blob/master/docs/api_versioning.md
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import struct
import sys
import tarfile

OBS_ARCHIVE_URL = "https://cdn-fastly.obsproject.com/downloads/cef_binary_7871_macos_arm64.tar.xz"
OBS_ARCHIVE_SHA256 = "0343233fc462384c2f1f58a4222028bc6171f05948a822241f89df2faa7cee8d"
ACTUAL_CEF_VERSION = "150.0.19+gcf60f42+chromium-150.0.7871.252"
BINDINGS_CEF_VERSION = "150.0.14+g7c1aa68+chromium-150.0.7871.129"
MAC_STABLE_HASH = "e85c868aa8a947e1e7c9904c0745c60ae490fa05"
MAC_EXPERIMENTAL_HASH = "0934859000c911cdd75907f63f2ef8e485706e5b"
FRAMEWORK = "Chromium Embedded Framework.framework"
PROJECT_CACHE = Path(__file__).resolve().parents[1] / ".cache" / "cef-obs"
MAX_ARCHIVE_BYTES = 512 * 1024 * 1024
MAX_EXPANDED_BYTES = 2 * 1024 * 1024 * 1024


class PrepareError(Exception):
    pass


def hash_file(path: Path, *, limit: int = MAX_EXPANDED_BYTES) -> tuple[str, str, int]:
    sha256, sha1 = hashlib.sha256(), hashlib.sha1()
    size = 0
    with path.open("rb") as incoming:
        while chunk := incoming.read(1024 * 1024):
            size += len(chunk)
            if size > limit:
                raise PrepareError("file_size_limit")
            sha256.update(chunk)
            sha1.update(chunk)
    return sha256.hexdigest(), sha1.hexdigest(), size


def mac_api_hash(header: str, version: str) -> str | None:
    marker = f"#define CEF_API_VERSION_{version} {version}"
    if marker not in header:
        return None
    section = header.split(marker, 1)[1].split("#define CEF_API_VERSION_", 1)[0]
    if "#elif defined(OS_MAC)" not in section:
        return None
    mac = section.split("#elif defined(OS_MAC)", 1)[1]
    match = re.search(rf'#define\s+CEF_API_HASH_{re.escape(version)}\s+"([0-9a-f]{{40}})"', mac)
    return match.group(1) if match else None


def validate_api_headers(api_header: str, version_header: str) -> None:
    if not re.search(r'^#define\s+CEF_VERSION\s+"' + re.escape(ACTUAL_CEF_VERSION) + r'"\s*$', version_header, re.MULTILINE):
        raise PrepareError("sdk_version_mismatch")
    if not re.search(r"^#define\s+CEF_API_VERSION_LAST\s+CEF_API_VERSION_15000\s*$", api_header, re.MULTILINE):
        raise PrepareError("sdk_api_version_mismatch")
    if mac_api_hash(api_header, "15000") != MAC_STABLE_HASH:
        raise PrepareError("sdk_stable_api_hash_mismatch")
    if mac_api_hash(api_header, "999999") != MAC_EXPERIMENTAL_HASH:
        raise PrepareError("sdk_experimental_api_hash_mismatch")


def verify_archive_files(archive: Path, expected: dict[str, str]) -> None:
    """Check the SDK against the verified archive without extracting/copying it."""
    remaining = set(expected)
    expanded = 0
    with tarfile.open(archive, "r|xz") as incoming:
        for index, member in enumerate(incoming):
            if index > 20_000:
                raise PrepareError("archive_member_limit")
            path = PurePosixPath(member.name)
            if path.is_absolute() or ".." in path.parts:
                raise PrepareError("unsafe_archive_member")
            expanded += member.size
            if expanded > MAX_EXPANDED_BYTES:
                raise PrepareError("archive_expansion_limit")
            name = path.as_posix()
            if name not in remaining:
                continue
            if not member.isfile():
                raise PrepareError("sdk_archive_file_type_mismatch")
            content = incoming.extractfile(member)
            if content is None:
                raise PrepareError("archive_read_failure")
            digest = hashlib.sha256()
            while chunk := content.read(1024 * 1024):
                digest.update(chunk)
            if digest.hexdigest() != expected[name]:
                raise PrepareError("sdk_archive_content_mismatch")
            remaining.remove(name)
            if not remaining:
                break
    if remaining:
        raise PrepareError("sdk_archive_files_missing")


def prepare(archive_path: Path, sdk_path: Path, *, check: bool = False, cache_root: Path | None = None, expected_sha256: str | None = None) -> dict:
    if not archive_path.is_absolute() or not sdk_path.is_absolute():
        raise PrepareError("absolute_paths_required")
    archive = archive_path.resolve(strict=True)
    sdk = sdk_path.resolve(strict=True)
    cache = (cache_root or PROJECT_CACHE).resolve(strict=True)
    if sdk == cache or cache not in sdk.parents or not sdk.is_dir() or not archive.is_file():
        raise PrepareError("sdk_must_be_project_obs_cache")
    archive_hash, archive_sha1, archive_bytes = hash_file(archive, limit=MAX_ARCHIVE_BYTES)
    if archive_hash != (expected_sha256 or OBS_ARCHIVE_SHA256):
        raise PrepareError("obs_archive_sha256_mismatch")
    for relative in ["CMakeLists.txt", "LICENSE.txt", "CREDITS.html", "include/cef_api_versions.h", "include/cef_version.h"]:
        path = sdk / relative
        if not path.is_file() or sdk not in path.resolve().parents:
            raise PrepareError("sdk_layout_incomplete")
    if not (sdk / "cmake").is_dir() or not (sdk / "libcef_dll").is_dir():
        raise PrepareError("sdk_layout_incomplete")
    api_header = (sdk / "include/cef_api_versions.h").read_text(encoding="utf-8")
    version_header = (sdk / "include/cef_version.h").read_text(encoding="utf-8")
    validate_api_headers(api_header, version_header)
    release_framework = sdk / "Release" / FRAMEWORK
    if release_framework.is_symlink() or not release_framework.is_dir():
        raise PrepareError("sdk_framework_missing")
    binary = (release_framework / "Chromium Embedded Framework").resolve(strict=True)
    if sdk not in binary.parents or not binary.is_file():
        raise PrepareError("sdk_framework_escapes_cache")
    with binary.open("rb") as incoming:
        magic = incoming.read(8)
    if len(magic) != 8 or magic[:4] != b"\xcf\xfa\xed\xfe" or struct.unpack_from("<I", magic, 4)[0] != 0x0100000C:
        raise PrepareError("sdk_framework_not_arm64")
    expected = {relative: hash_file(sdk / relative)[0] for relative in ["include/cef_api_versions.h", "include/cef_version.h", "CMakeLists.txt"]}
    binary_hash = hash_file(binary)[0]
    expected[binary.relative_to(sdk).as_posix()] = binary_hash
    verify_archive_files(archive, expected)
    alias = sdk / FRAMEWORK
    relative_framework = Path("Release") / FRAMEWORK
    if alias.exists() or alias.is_symlink():
        if not alias.is_symlink() or alias.readlink() != relative_framework or alias.resolve() != release_framework.resolve():
            raise PrepareError("existing_framework_alias_conflict")
    metadata = {
        "type": "standard",
        "name": f"cef_binary_{ACTUAL_CEF_VERSION}_macosarm64.tar.xz",
        "sha1": archive_sha1,
        "lume_obs_provenance": {
            "source_url": OBS_ARCHIVE_URL,
            "source_sha256": archive_hash,
            "sdk_cef_version": ACTUAL_CEF_VERSION,
            "bindings_cef_version": BINDINGS_CEF_VERSION,
            "stable_api_version": 15000,
            "stable_mac_api_hash": MAC_STABLE_HASH,
            "experimental_api_version": 999999,
            "experimental_mac_api_hash": MAC_EXPERIMENTAL_HASH,
            "framework_sha256": binary_hash,
            "verified_against_original_archive": sorted(expected),
        },
    }
    if not check:
        if not alias.is_symlink():
            alias.symlink_to(relative_framework, target_is_directory=True)
        metadata_path = sdk / "archive.json"
        if metadata_path.is_symlink():
            raise PrepareError("archive_metadata_symlink_rejected")
        metadata_path.write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
    return {
        "status": "checked" if check else "prepared",
        "sdk_cef_version": ACTUAL_CEF_VERSION,
        "bindings_cef_version": BINDINGS_CEF_VERSION,
        "platform": "macosarm64",
        "source_sha256": archive_hash,
        "archive_bytes": archive_bytes,
        "stable_api_version": 15000,
        "experimental_api_version": 999999,
        "api_hashes_match": True,
        "framework_sha256": binary_hash,
        "original_archive_files_verified": len(expected),
        "framework_duplicated": False,
        "codec_support_verified": False,
        "drm_verified": False,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--archive", required=True, type=Path, help="Original official cef_binary_7871_macos_arm64.tar.xz")
    parser.add_argument("--sdk", required=True, type=Path, help="Already extracted SDK inside .cache/cef-obs")
    parser.add_argument("--check", action="store_true", help="Validate without adding framework alias/archive.json")
    args = parser.parse_args(argv)
    try:
        result = prepare(args.archive, args.sdk, check=args.check)
    except PrepareError as error:
        print(json.dumps({"status": "error", "error": str(error)}))
        return 2
    except (OSError, ValueError, UnicodeError, tarfile.TarError, struct.error):
        print(json.dumps({"status": "error", "error": "filesystem_or_archive_error"}))
        return 2
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
