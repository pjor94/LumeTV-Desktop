#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Stage an explicitly supplied Widevine component into a dedicated CEF profile.

This copies an existing component; it never downloads, executes, patches, or
redistributes it, and never changes the source installation. Preparing the files
does not prove CDM loading, codec availability, VMP, or successful DRM playback.

Registration layout is based on the upstream implementations:
https://github.com/chromium/chromium/blob/main/components/component_updater/component_installer.cc
  SelectComponentVersion scans WidevineCdm/<version> and validates the manifest.
https://github.com/chromium/chromium/blob/main/chrome/browser/component_updater/widevine_cdm_component_installer.cc
  VerifyInstallation checks the platform-specific library and CDM manifest.
https://github.com/chromium/chromium/blob/main/chrome/common/media/component_widevine_cdm_hint_file_linux.cc
  Linux hint JSON has a required Path and an optional LastBundledVersion.
https://github.com/chromiumembedded/cef/blob/master/patch/patches/linux_chrome_widevine_3149.patch
  CEF places that Linux hint under DIR_USER_DATA/WidevineCdm.

Run before CEF starts, using exactly its root_cache_path as --profile. The
profile must already contain the regular marker file .lume-player-profile.
Only manifest, license text, and the current platform library/signature are
copied. No host verification signatures are generated or substituted.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import stat
import struct
import sys
import tempfile

MAX_COMPONENT_BYTES = 64 * 1024 * 1024
MAX_MANIFEST_BYTES = 256 * 1024
PROFILE_MARKER = ".lume-player-profile"
HINT_NAME = "latest-component-updated-widevine-cdm"
VERSION = re.compile(r"[0-9]+(?:\.[0-9]+){3}\Z", re.ASCII)


class SeedError(Exception):
    """An allowlisted error code, without source contents or OS path details."""


@dataclass(frozen=True)
class ComponentFile:
    relative: Path
    size: int
    sha256: str


@dataclass(frozen=True)
class SeedPlan:
    source: Path
    profile: Path
    destination: Path
    version: str
    platform_key: str
    library_name: str
    files: tuple[ComponentFile, ...]


def platform_layout(system: str, machine: str) -> tuple[str, str, str, str]:
    arch = {
        "arm64": "arm64", "aarch64": "arm64",
        "x86_64": "x64", "amd64": "x64", "x64": "x64",
        "x86": "x86", "i386": "x86", "i686": "x86",
    }.get(machine.lower())
    os_name = {"darwin": "mac", "windows": "win", "linux": "linux"}.get(system.lower())
    if not arch or not os_name or (os_name == "mac" and arch == "x86"):
        raise SeedError("unsupported_platform")
    library = {"mac": "libwidevinecdm.dylib", "win": "widevinecdm.dll", "linux": "libwidevinecdm.so"}[os_name]
    return f"{os_name}_{arch}", library, os_name, arch


def regular_file(root: Path, relative: Path, limit: int) -> Path:
    """Reject links and special files at every component below a trusted root."""
    if relative.is_absolute() or ".." in relative.parts:
        raise SeedError("unsafe_component_path")
    current = root
    for index, part in enumerate(relative.parts):
        current = current / part
        try:
            info = current.lstat()
        except OSError:
            raise SeedError("component_file_missing") from None
        if stat.S_ISLNK(info.st_mode):
            raise SeedError("component_symlink_rejected")
        if index < len(relative.parts) - 1:
            if not stat.S_ISDIR(info.st_mode):
                raise SeedError("invalid_component_directory")
        elif not stat.S_ISREG(info.st_mode) or info.st_size > limit:
            raise SeedError("invalid_or_oversized_component_file")
    return current


def digest_file(path: Path, limit: int) -> tuple[int, str]:
    digest = hashlib.sha256()
    size = 0
    # O_NOFOLLOW protects against a last-component link swap on supported OSes.
    descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    with os.fdopen(descriptor, "rb") as source:
        if not stat.S_ISREG(os.fstat(source.fileno()).st_mode):
            raise SeedError("invalid_component_file")
        while chunk := source.read(128 * 1024):
            size += len(chunk)
            if size > limit:
                raise SeedError("component_size_limit")
            digest.update(chunk)
    return size, digest.hexdigest()


def validate_library(path: Path, os_name: str, arch: str) -> None:
    with path.open("rb") as library:
        head = library.read(4096)
    expected = {
        "mac": {"arm64": 0x0100000C, "x64": 0x01000007},
        "linux": {"arm64": 183, "x64": 62, "x86": 3},
        "win": {"arm64": 0xAA64, "x64": 0x8664, "x86": 0x014C},
    }[os_name][arch]
    found = False
    if os_name == "mac" and len(head) >= 8:
        endian = {b"\xcf\xfa\xed\xfe": "<", b"\xce\xfa\xed\xfe": "<",
                  b"\xfe\xed\xfa\xcf": ">", b"\xfe\xed\xfa\xce": ">"}.get(head[:4])
        if endian:
            found = struct.unpack_from(endian + "I", head, 4)[0] == expected
        elif head[:4] in (b"\xca\xfe\xba\xbe", b"\xca\xfe\xba\xbf"):
            count = struct.unpack_from(">I", head, 4)[0]
            stride = 32 if head[:4] == b"\xca\xfe\xba\xbf" else 20
            if count <= 32 and len(head) >= 8 + count * stride:
                found = any(struct.unpack_from(">I", head, 8 + i * stride)[0] == expected for i in range(count))
    elif os_name == "linux" and len(head) >= 20 and head[:4] == b"\x7fELF":
        endian = {1: "<", 2: ">"}.get(head[5])
        expected_class = 1 if arch == "x86" else 2
        found = bool(endian and head[4] == expected_class and struct.unpack_from(endian + "H", head, 18)[0] == expected)
    elif os_name == "win" and len(head) >= 64 and head[:2] == b"MZ":
        offset = struct.unpack_from("<I", head, 0x3C)[0]
        found = bool(offset + 6 <= len(head) and head[offset:offset + 4] == b"PE\0\0" and struct.unpack_from("<H", head, offset + 4)[0] == expected)
    if not found:
        raise SeedError("library_architecture_mismatch")


def validate_profile(path: Path) -> Path:
    if not path.is_absolute() or path.is_symlink():
        raise SeedError("profile_must_be_absolute_directory")
    try:
        profile = path.resolve(strict=True)
    except OSError:
        raise SeedError("profile_missing") from None
    if not profile.is_dir():
        raise SeedError("profile_must_be_absolute_directory")
    try:
        regular_file(profile, Path(PROFILE_MARKER), 4096)
    except SeedError:
        raise SeedError("dedicated_profile_marker_required") from None
    return profile


def prepare(source_path: Path, profile_path: Path, *, system: str | None = None, machine: str | None = None) -> SeedPlan:
    if not source_path.is_absolute():
        raise SeedError("source_must_be_absolute_directory")
    try:
        source = source_path.resolve(strict=True)
    except OSError:
        raise SeedError("source_missing") from None
    if not source.is_dir():
        raise SeedError("source_must_be_absolute_directory")
    profile = validate_profile(profile_path)
    if source == profile or source in profile.parents or profile in source.parents:
        raise SeedError("source_profile_overlap")
    platform_key, library_name, os_name, arch = platform_layout(system or platform.system(), machine or platform.machine())
    manifest_path = regular_file(source, Path("manifest.json"), MAX_MANIFEST_BYTES)
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (ValueError, UnicodeError):
        raise SeedError("invalid_manifest") from None
    if not isinstance(manifest, dict) or manifest.get("name") != "WidevineCdm" or manifest.get("manifest_version") != 2:
        raise SeedError("invalid_widevine_manifest")
    version = manifest.get("version")
    if not isinstance(version, str) or not VERSION.fullmatch(version) or any(int(part) > 0xFFFFFFFF for part in version.split(".")):
        raise SeedError("invalid_component_version")
    declared_platforms = manifest.get("platforms")
    if declared_platforms is not None and (not isinstance(declared_platforms, list) or not any(
        isinstance(entry, dict) and entry.get("os") == os_name and entry.get("arch") == arch
        and entry.get("sub_package_path", f"_platform_specific/{platform_key}/") == f"_platform_specific/{platform_key}/"
        for entry in declared_platforms
    )):
        raise SeedError("manifest_platform_mismatch")
    platform_relative = Path("_platform_specific") / platform_key
    library_relative = platform_relative / library_name
    library_path = regular_file(source, library_relative, MAX_COMPONENT_BYTES)
    validate_library(library_path, os_name, arch)
    allowed_platform = {library_name, library_name + ".sig"}
    if any(entry.name not in allowed_platform for entry in (source / platform_relative).iterdir()):
        raise SeedError("unexpected_platform_file")
    allowed_root = {"manifest.json", "LICENSE", "LICENSE.txt", "_platform_specific", "_metadata", "imgs"}
    if any(entry.name not in allowed_root for entry in source.iterdir()):
        raise SeedError("unexpected_component_file")
    relative_files = [Path("manifest.json"), library_relative]
    for optional in [Path("LICENSE"), Path("LICENSE.txt"), Path(str(library_relative) + ".sig")]:
        if (source / optional).exists() or (source / optional).is_symlink():
            relative_files.append(optional)
    files = []
    total = 0
    for relative in relative_files:
        file_path = regular_file(source, relative, MAX_COMPONENT_BYTES)
        size, checksum = digest_file(file_path, MAX_COMPONENT_BYTES)
        total += size
        if total > MAX_COMPONENT_BYTES:
            raise SeedError("component_size_limit")
        files.append(ComponentFile(relative, size, checksum))
    return SeedPlan(source, profile, profile / "WidevineCdm" / version, version, platform_key, library_name, tuple(files))


def existing_component(plan: SeedPlan) -> bool:
    base = plan.destination.parent
    if base.is_symlink() or (base.exists() and not base.is_dir()):
        raise SeedError("unsafe_component_destination")
    if base.exists():
        for entry in base.iterdir():
            if entry.name == HINT_NAME:
                regular_file(base, Path(HINT_NAME), 4096)
            elif entry.name != plan.version:
                raise SeedError("profile_has_other_component_entries")
    if plan.destination.is_symlink() or (plan.destination.exists() and not plan.destination.is_dir()):
        raise SeedError("unsafe_component_destination")
    if not plan.destination.exists():
        return False
    for item in plan.files:
        path = regular_file(plan.destination, item.relative, MAX_COMPONENT_BYTES)
        if digest_file(path, MAX_COMPONENT_BYTES) != (item.size, item.sha256):
            raise SeedError("existing_component_conflict")
    return True


def write_linux_hint(plan: SeedPlan) -> None:
    hint = plan.destination.parent / HINT_NAME
    if hint.exists() or hint.is_symlink():
        regular_file(plan.destination.parent, Path(HINT_NAME), 4096)
        try:
            previous = json.loads(hint.read_text(encoding="utf-8"))
        except (ValueError, UnicodeError):
            raise SeedError("existing_hint_invalid") from None
        if not isinstance(previous, dict) or previous.get("Path") != str(plan.destination):
            raise SeedError("existing_hint_conflict")
    descriptor, temporary_name = tempfile.mkstemp(prefix=".lume-hint-", dir=hint.parent)
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as output:
            json.dump({"Path": str(plan.destination)}, output)
            output.write("\n")
        temporary.replace(hint)
    finally:
        temporary.unlink(missing_ok=True)


def execute(plan: SeedPlan, *, check: bool = False) -> dict:
    exists = existing_component(plan)
    if not check and not exists:
        base = plan.destination.parent
        base.mkdir(mode=0o700, exist_ok=True)
        temporary = Path(tempfile.mkdtemp(prefix=".lume-seed-", dir=base))
        try:
            for item in plan.files:
                source = regular_file(plan.source, item.relative, MAX_COMPONENT_BYTES)
                destination = temporary / item.relative
                destination.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
                # Copy only the validated allowlist. Never copy a browser bundle or profile.
                with source.open("rb") as incoming, destination.open("xb") as outgoing:
                    remaining = item.size
                    while chunk := incoming.read(min(128 * 1024, remaining + 1)):
                        remaining -= len(chunk)
                        if remaining < 0:
                            raise SeedError("source_changed_during_copy")
                        outgoing.write(chunk)
                if digest_file(destination, MAX_COMPONENT_BYTES) != (item.size, item.sha256):
                    raise SeedError("source_changed_during_copy")
                destination.chmod(0o755 if item.relative.name == plan.library_name else 0o644)
            if plan.destination.exists() or plan.destination.is_symlink():
                raise SeedError("destination_changed_during_copy")
            temporary.rename(plan.destination)
        finally:
            if temporary.exists():
                shutil.rmtree(temporary)
    linux = plan.platform_key.startswith("linux_")
    if not check and linux:
        write_linux_hint(plan)
    return {
        "status": "ready_to_seed" if check else "already_seeded" if exists else "seeded",
        "version": plan.version,
        "platform": plan.platform_key,
        "source_external": True,
        "component_prepared": exists or not check,
        "cdm_loaded": False,
        "drm_verified": False,
        "host_verification_signatures_provided": False,
        "linux_hint_required": linux,
        "linux_hint_prepared": linux and not check,
        "bytes": sum(item.size for item in plan.files),
        "files": [{"path": item.relative.as_posix(), "bytes": item.size, "sha256": item.sha256} for item in plan.files],
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--source", required=True, type=Path, help="Existing WidevineCdm directory containing manifest.json")
    parser.add_argument("--profile", required=True, type=Path, help="Dedicated CEF root profile with .lume-player-profile marker")
    parser.add_argument("--check", action="store_true", help="Validate and hash only; do not create or copy files")
    args = parser.parse_args(argv)
    try:
        result = execute(prepare(args.source, args.profile), check=args.check)
    except SeedError as error:
        print(json.dumps({"status": "error", "error": str(error), "source_external": True}))
        return 2
    except (OSError, ValueError, OverflowError, struct.error):
        print(json.dumps({"status": "error", "error": "filesystem_or_format_error", "source_external": True}))
        return 2
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
