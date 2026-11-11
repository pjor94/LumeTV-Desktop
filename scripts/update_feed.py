#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Sign the desktop update manifest. Private keys never enter release artifacts."""
from __future__ import annotations
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
PUBLIC_KEY = ROOT / "resources/update-public-key.hex"
SPKI = bytes.fromhex("302a300506032b6570032100")


def public_key(private: Path) -> bytes:
    value = subprocess.run(["openssl", "pkey", "-in", str(private), "-pubout", "-outform", "DER"],
                           check=True, capture_output=True).stdout
    if len(value) != 44 or not value.startswith(SPKI):
        raise ValueError("The update key must use Ed25519")
    return value[len(SPKI):]


def initialize(private: Path, public: Path) -> None:
    if private.exists() or public.exists():
        raise ValueError("Refusing to replace an existing update signing key")
    private.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    descriptor = os.open(private, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    os.close(descriptor)
    subprocess.run(["openssl", "genpkey", "-algorithm", "ED25519", "-out", str(private)],
                   check=True, capture_output=True)
    key = public_key(private)
    public.parent.mkdir(parents=True, exist_ok=True)
    public.write_text(key.hex() + "\n")
    print("Update public-key fingerprint: " + hashlib.sha256(key).hexdigest())


def build_manifest(directory: Path, version: str, prerelease: bool, notes: str) -> dict:
    targets = {}
    for platform in ("macos-arm64", "macos-x64", "windows-x64", "linux-x64"):
        metadata = json.loads((directory / f"LumeTV-{version}-{platform}.build.json").read_text())
        if metadata["version"] != version or metadata["platform"] != platform:
            raise ValueError("Update metadata differs from the release")
        artifacts = metadata["artifacts"]
        extension = ".tar.gz" if platform == "linux-x64" else ".zip"
        target = {}
        for role, suffix in [("archive", extension), ("installer", ".dmg" if platform.startswith("macos-") else ".deb")]:
            if role == "installer" and platform == "windows-x64":
                continue
            name = f"LumeTV-{version}-{platform}{suffix}"
            matches = [item for item in artifacts if item["name"] == name]
            if len(matches) != 1:
                raise ValueError("Missing update artifact: " + name)
            item = matches[0]
            path = directory / name
            with path.open("rb") as stream:
                digest = hashlib.file_digest(stream, "sha256").hexdigest()
            if path.is_symlink() or path.stat().st_size != item["size"] or digest != item["sha256"]:
                raise ValueError("Update artifact checksum differs: " + name)
            target[role] = item
        targets[platform] = target
    return {"schema": 1, "application_id": "it.lumetv.desktop", "repository": "pjor94/LumeTV-Desktop",
            "version": version, "prerelease": prerelease, "notes": notes[:4000], "targets": targets}


def sign_manifest(manifest: dict, directory: Path, private_pem: str, public: Path = PUBLIC_KEY) -> list[Path]:
    raw = (json.dumps(manifest, sort_keys=True, separators=(",", ":"), ensure_ascii=False) + "\n").encode()
    if len(raw) > 65536:
        raise ValueError("Update manifest is too large")
    with tempfile.TemporaryDirectory(prefix="lume-update-sign-") as temp:
        key = Path(temp) / "signing.pem"
        key.write_text(private_pem)
        key.chmod(0o600)
        if public_key(key) != bytes.fromhex(public.read_text().strip()):
            raise ValueError("Signing key differs from the key embedded in Lume")
        payload = Path(temp) / "payload.json"
        payload.write_bytes(raw)
        signature = subprocess.run(["openssl", "pkeyutl", "-sign", "-rawin", "-inkey", str(key), "-in", str(payload)],
                                   check=True, capture_output=True).stdout
    if len(signature) != 64:
        raise ValueError("Invalid Ed25519 signature size")
    output = [directory / "update.json", directory / "update.sig"]
    output[0].write_bytes(raw)
    output[1].write_text(base64.b64encode(signature).decode() + "\n")
    return output


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["init"])
    parser.add_argument("--private", type=Path, default=ROOT / "release-keys/updates-ed25519.pem")
    parser.add_argument("--public", type=Path, default=PUBLIC_KEY)
    args = parser.parse_args()
    initialize(args.private, args.public)
