#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Check the source snapshot intended for GitHub without reading ignored data.

By default inspect tracked and non-ignored untracked files; --tracked checks only
the Git index (used by CI). Findings contain paths and line numbers, never the
matched credentials. This is an offline snapshot check, not a history audit.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys
from urllib.parse import unquote, urlsplit


MAX_FILE_BYTES = 5 * 1024 * 1024
PRIVATE_DIRS = {
    ".aws", ".cache", ".git", ".ssh", ".venv", "__pycache__", "profiles",
    "target", "WidevineCdm",
}
BUILD_DIRS = {"artifacts", "build", "coverage", "dist", "releases", "reports"}
PRIVATE_SUFFIXES = {
    ".dll", ".dmg", ".dylib", ".exe", ".key", ".log", ".mobileprovision",
    ".msi", ".p12", ".pem", ".pfx", ".pkg", ".pyc", ".so",
}
IMAGE_SUFFIXES = {".png", ".jpg", ".jpeg", ".icns"}
REQUIRED_FILES = {
    ".gitattributes", ".gitignore", "Cargo.toml", "Cargo.lock", "LICENSE",
    "README.md", "THIRD_PARTY_NOTICES.md", "resources/channels.m3u",
    "resources/catalog-legacy.json", "vendor/download-cef/LICENSE-APACHE",
    "vendor/download-cef/LICENSE-MIT", "vendor/download-cef/LUME_PATCH.md",
    "vendor/shaka/LICENSE", "vendor/shaka/dist/shaka-player.compiled.js",
    "vendor/download-cef/lume-sdk-pins.json", "crates/lume-cef-windows/Cargo.toml",
    "crates/lume-cef-windows/src/lib.rs", "scripts/prepare_cef_sdk.py",
    "scripts/build_desktop.py", "scripts/release_desktop.py", "docs/RELEASE.md",
    ".github/workflows/packages.yml", ".github/workflows/release.yml",
}
SECRET_PATTERNS = {
    "private signing key": re.compile(
        r"-----BEGIN (?:RSA |EC |OPENSSH |ENCRYPTED )?PRIVATE KEY-----"
    ),
    "GitHub credential": re.compile(
        r"\b(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,})\b"
    ),
    "AWS access key": re.compile(r"\b(?:AKIA|ASIA)[A-Z0-9]{16}\b"),
    "Slack credential": re.compile(r"\bxox[baprs]-[A-Za-z0-9-]{24,}\b"),
}
LINK = re.compile(r"!?\[[^\]\n]*\]\(([^\n]+?)\)")
IGNORED_SAMPLES = (
    "target/debug/lume-player", ".cache/sdk/archive.tar.xz",
    "profiles/test/browser/Local State", "profiles/test/preferences.json",
    "reports/raw.json", "reports/run.log", ".env", ".env.production",
    "certs/signing.p12", "certs/signing.key", "WidevineCdm/manifest.json",
    "nested/libwidevinecdm.dylib", "dist/Lume TV.app/Contents/Info.plist",
)


def forbidden_path(name: str) -> bool:
    path = PurePosixPath(name)
    if name == "reports/.gitkeep":
        return False
    return (
        path.is_absolute() or ".." in path.parts
        or any(part in PRIVATE_DIRS or part.endswith(".app") for part in path.parts)
        or path.parts[0] in BUILD_DIRS
        or path.suffix.lower() in PRIVATE_SUFFIXES
        or path.name in {".DS_Store", "Thumbs.db"}
        or path.name.startswith("._")
        or (path.name.startswith(".env") and not path.name.endswith(".example"))
        or path.name.lower().startswith("libwidevinecdm.")
        or path.name.lower() == "widevinecdm.dll"
    )


def markdown_links(text: str):
    # Ignore examples in fenced code; preserve ordinary inline code labels.
    text = re.sub(r"^(`{3,}|~{3,})[^\n]*\n.*?^\1[^\n]*$", "", text,
                  flags=re.MULTILINE | re.DOTALL)
    for match in LINK.finditer(text):
        destination = match.group(1).strip()
        if destination.startswith("<"):
            destination = destination[1:].split(">", 1)[0]
        else:
            destination = destination.split(' "', 1)[0]
        yield destination


def audit_files(root: Path, names: list[str]) -> list[str]:
    root = root.resolve()
    public = set(names)
    findings = []
    for name in sorted(public):
        if forbidden_path(name):
            findings.append(f"{name}: private or generated file in source snapshot")
            continue
        path = root / name
        if path.is_symlink() or not path.resolve().is_relative_to(root):
            findings.append(f"{name}: source symlink or path outside repository")
            continue
        if not path.is_file():
            findings.append(f"{name}: missing source file")
            continue
        if path.stat().st_size > MAX_FILE_BYTES:
            findings.append(f"{name}: exceeds source file limit (5 MiB)")
            continue
        if path.suffix.lower() in IMAGE_SUFFIXES:
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except (UnicodeError, OSError):
            findings.append(f"{name}: unexpected binary or unreadable source file")
            continue
        if "\0" in text:
            findings.append(f"{name}: unexpected binary source file")
            continue
        for line_number, line in enumerate(text.splitlines(), 1):
            for label, pattern in SECRET_PATTERNS.items():
                if pattern.search(line):
                    findings.append(f"{name}:{line_number}: possible {label}")
        if name.startswith("docs/evidence/") and path.suffix == ".json":
            try:
                evidence = json.loads(text)
                private_fields = {
                    "token", "headers", "license_url", "license_challenge",
                    "license_response", "cookies", "profile", "browser_path",
                }

                def has_private_fields(value):
                    if isinstance(value, dict):
                        return bool(private_fields.intersection(value)) or any(
                            has_private_fields(item) for item in value.values()
                        )
                    return isinstance(value, list) and any(map(has_private_fields, value))

                if re.search(r"https?://", text) or has_private_fields(evidence):
                    findings.append(f"{name}: private metadata or URL in public evidence")
            except ValueError:
                findings.append(f"{name}: invalid evidence JSON")
        if path.suffix.lower() != ".md" or name.startswith("vendor/"):
            continue  # Upstream documentation is preserved without rewriting it.
        if re.search(r"/(?:Users/[^/\s]+|Volumes/[^/\s]+/Developer)/", text):
            findings.append(f"{name}: machine-specific path in public documentation")
        for destination in markdown_links(text):
            target = urlsplit(destination)
            if target.scheme in {"http", "https", "mailto"} or destination.startswith("#"):
                continue
            if target.scheme or target.netloc:
                findings.append(f"{name}: non-portable documentation link")
                continue
            resolved = (path.parent / unquote(target.path)).resolve()
            if not resolved.is_relative_to(root):
                findings.append(f"{name}: documentation link outside repository")
                continue
            relative = resolved.relative_to(root).as_posix()
            if relative not in public and not any(p.startswith(relative + "/") for p in public):
                findings.append(f"{name}: link target absent from source snapshot: {relative}")
    return findings


def git_files(root: Path, *, tracked_only: bool) -> list[str]:
    command = ["git", "ls-files", "-z", "--cached"]
    if not tracked_only:
        command.extend(["--others", "--exclude-standard"])
    result = subprocess.run(command, cwd=root, check=True, capture_output=True)
    return sorted(set(result.stdout.decode("utf-8").split("\0")) - {""})


def index_matches_worktree(root: Path) -> bool:
    result = subprocess.run(
        ["git", "diff", "--quiet", "--exit-code", "--"], cwd=root, capture_output=True,
    )
    if result.returncode not in (0, 1):
        raise subprocess.CalledProcessError(result.returncode, "git diff")
    return result.returncode == 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tracked", action="store_true", help="Inspect the Git index only")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    try:
        if args.tracked and not index_matches_worktree(root):
            print("Tracked files differ from the index. Review and stage changes before --tracked.")
            return 1
        names = git_files(root, tracked_only=args.tracked)
        ignore_check = subprocess.run(
            ["git", "check-ignore", "--no-index", "-z", "--stdin"], cwd=root,
            input=("\0".join(IGNORED_SAMPLES) + "\0").encode(), capture_output=True,
        )
        if ignore_check.returncode not in (0, 1):
            raise subprocess.CalledProcessError(ignore_check.returncode, "git check-ignore")
    except (OSError, subprocess.CalledProcessError, UnicodeError):
        print("Run this check inside a Git checkout with Git installed.", file=sys.stderr)
        return 2
    findings = audit_files(root, names)
    findings.extend(f"{name}: required source or license missing from snapshot"
                    for name in sorted(REQUIRED_FILES - set(names)))
    ignored = set(ignore_check.stdout.decode().split("\0"))
    findings.extend(f"{name}: local data sample is not protected by .gitignore"
                    for name in IGNORED_SAMPLES if name not in ignored)
    shaka = root / "vendor/shaka/dist/shaka-player.compiled.js"
    expected = "61e5fb69716a6971992616cffbfdbc75d3afadcd881295243de7b6f12ddb0f26"
    if not shaka.is_file() or hashlib.sha256(shaka.read_bytes()).hexdigest() != expected:
        findings.append("vendor/shaka: compiled artifact does not match the documented checksum")
    for finding in findings:
        print(finding)
    if findings:
        print(f"Repository check failed: {len(findings)} finding(s).")
        return 1
    size = sum((root / name).stat().st_size for name in names)
    print(f"Repository check passed: {len(names)} files, {size / 1024 / 1024:.2f} MiB.")
    print("Ignored local data, source paths, notices, Shaka checksum and Markdown targets checked.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
