#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Build and package the complete native app, not just the Rust CLI.

Produces macOS DMG/ZIP, Windows portable ZIP, Linux tar.gz/Ubuntu 24.04 DEB.
No provider is contacted. The native smoke check only asks for runtime metadata.
SDK pins, sandbox bootstrap, notices, hashes and source revision accompany builds.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import platform as host_platform
import re
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
import zipfile

import bundle_cef
import prepare_cef_sdk as cef_sdk

ROOT = Path(__file__).resolve().parents[1]
LINUX_DEPS = (
    "libasound2t64", "libatk-bridge2.0-0t64", "libatk1.0-0t64", "libcups2t64",
    "libdbus-1-3", "libdrm2", "libgbm1", "libglib2.0-0t64", "libgtk-3-0t64",
    "libnspr4", "libnss3", "libx11-6", "libxcb1", "libxcomposite1",
    "libxdamage1", "libxext6", "libxfixes3", "libxkbcommon0", "libxrandr2",
)


def run(command: list[str], *, cwd: Path = ROOT, env: dict | None = None, capture=False) -> str:
    result = subprocess.run(command, cwd=cwd, env=env, check=True,
                            text=True, stdout=subprocess.PIPE if capture else None)
    return result.stdout if capture else ""


def app_version(root: Path = ROOT) -> str:
    version = tomllib.loads((root / "Cargo.toml").read_text())["package"]["version"]
    if not re.fullmatch(r"\d+\.\d+\.\d+(?:-[A-Za-z0-9.-]+)?", version):
        raise ValueError("Unsupported application version")
    return version


def source_revision(root: Path = ROOT) -> str:
    result = subprocess.run(["git", "rev-parse", "--verify", "HEAD"], cwd=root,
                            capture_output=True, text=True)
    return result.stdout.strip() if result.returncode == 0 else "uncommitted"


def native_host(pin: dict) -> None:
    systems = {"Darwin": "macos", "Windows": "windows", "Linux": "linux"}
    arches = {"arm64": "aarch64", "aarch64": "aarch64", "AMD64": "x86_64", "x86_64": "x86_64"}
    if systems.get(host_platform.system()) != pin["os"] or arches.get(host_platform.machine()) != pin["arch"]:
        raise ValueError("Use a native runner matching the requested OS and architecture")


def validate_runtime_info(info: dict, pin: dict) -> None:
    expected = json.loads(cef_sdk.PIN_FILE.read_text())["bindings_cef_version"]
    if info.get("compiled_os") != pin["os"] or info.get("compiled_arch") != pin["arch"]:
        raise ValueError("Packaged executable does not match the requested native platform")
    if info.get("backend") != "embedded_cef" or not info.get("sandbox"):
        raise ValueError("Packaged executable lacks the embedded sandboxed runtime")
    if info.get("cef_expected", "").split("+", 1)[0] != expected or info.get("cef_api_version") != 999999:
        raise ValueError("Compiled CEF bindings/API differ from the SDK compatibility pins")


def collect_notices(sdk: Path, output: Path, *, cargo: list[str], env: dict,
                    root: Path = ROOT) -> None:
    output.mkdir(parents=True)
    for name in ("LICENSE", "THIRD_PARTY_NOTICES.md"):
        shutil.copy2(root / name, output / name)
    for source, name in [
        (sdk / "LICENSE.txt", "CEF-LICENSE.txt"), (sdk / "CREDITS.html", "CEF-CREDITS.html"),
        (sdk / "include/cef_version.h", "cef_version.h"),
        (root / "vendor/shaka/LICENSE", "Shaka-LICENSE"),
        (root / "vendor/download-cef/LICENSE-MIT", "download-cef-LICENSE-MIT"),
        (root / "vendor/download-cef/LICENSE-APACHE", "download-cef-LICENSE-APACHE"),
        (root / "vendor/download-cef/LUME_PATCH.md", "download-cef-LUME_PATCH.md"),
    ]:
        shutil.copy2(source, output / name)
    metadata = json.loads(run(cargo + ["metadata", "--locked", "--format-version=1", "--all-features"],
                              cwd=root, env=env, capture=True))
    inventory = []
    for package in sorted(metadata["packages"], key=lambda item: (item["name"], item["version"])):
        base = Path(package["manifest_path"]).parent
        destination = output / "rust" / (package["name"] + "-" + package["version"])
        if not package.get("license") and not package.get("license_file"):
            raise ValueError("Dependency has no declared license: " + package["name"])
        files = [p for p in base.iterdir() if p.is_file()
                 and p.name.upper().startswith(("LICENSE", "COPYING", "NOTICE"))]
        if package.get("license_file"):
            explicit = base / package["license_file"]
            if explicit.is_file() and explicit not in files:
                files.append(explicit)
        if files:
            destination.mkdir(parents=True)
            for source in files:
                shutil.copy2(source, destination / source.name)
        inventory.append({"name": package["name"], "version": package["version"],
                          "license": package.get("license"),
                          "notices": sorted(p.name for p in files)})
    (output / "Cargo-dependencies.json").write_text(json.dumps(inventory, indent=2) + "\n")


def copy_runtime(sdk: Path, destination: Path, os_name: str) -> None:
    patterns = ["*.pak", "*.dat", "*.bin", "vk_swiftshader_icd.json"]
    patterns += ["*.dll"] if os_name == "windows" else ["*.so", "*.so.*", "chrome-sandbox"]
    selected = {source for pattern in patterns for source in sdk.glob(pattern)}
    for source in sorted(selected):
        if "widevine" not in source.name.casefold():
            shutil.copy2(source, destination / source.name)
    for name in ("locales", "swiftshader"):
        if (sdk / name).is_dir():
            shutil.copytree(sdk / name, destination / name, ignore=bundle_cef.ignore_cdm)
    required = ["locales", "icudtl.dat", "resources.pak",
                "libcef.dll" if os_name == "windows" else "libcef.so"]
    if not all((destination / name).exists() for name in required):
        raise ValueError("Native package is missing CEF runtime resources")


def assemble(platform: str, sdk: Path, binaries: Path, stage: Path, notices: Path,
             *, root: Path = ROOT) -> tuple[Path, Path]:
    pin = cef_sdk.pins()[platform]
    version = app_version(root)
    if pin["os"] == "macos":
        payload = stage / "Lume TV.app"
        bundle_cef.build_bundle(sdk, binaries, payload, codec_build="custom",
                               project_root=root, version=version, additional_licenses=notices)
        executable = payload / "Contents/MacOS/lume-cef"
    else:
        payload = stage / f"LumeTV-{version}-{platform}"
        payload.mkdir(parents=True)
        copy_runtime(sdk, payload, pin["os"])
        shutil.copytree(notices, payload / "licenses")
        if pin["os"] == "windows":
            # SDK bootstrap and DLL stem must match. The bootstrap, not a normal
            # Rust executable, creates and owns CEF's Windows sandbox context.
            shutil.copy2(sdk / "bootstrap.exe", payload / "LumeTV.exe")
            shutil.copy2(binaries / "lume_cef_windows.dll", payload / "LumeTV.dll")
            shutil.copy2(binaries / "lume-player.exe", payload / "lume-player.exe")
            executable = payload / "LumeTV.exe"
            launch = "Avvia LumeTV.exe dalla cartella estratta."
        else:
            for name in ("lume-cef", "lume-player"):
                shutil.copy2(binaries / name, payload / name)
                (payload / name).chmod(0o755)
            executable = payload / "AppRun"
            executable.write_text('#!/bin/sh\nset -eu\nLUME_APP_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)\nexport LD_LIBRARY_PATH="$LUME_APP_DIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"\nexec "$LUME_APP_DIR/lume-cef" "$@"\n')
            executable.chmod(0o755)
            (payload / "chrome-sandbox").chmod(0o755)
            shutil.copy2(root / "web/app/assets/app-icon.png", payload / "lumetv.png")
            launch = ("Avvia AppRun. Richiede Linux x64 compatibile con Ubuntu 24.04 e le librerie di sistema elencate nella documentazione.\n"
                      "Il formato portabile richiede user namespace non privilegiati disponibili. Il pacchetto DEB installa anche l'helper sandbox CEF con i permessi previsti da Chromium.")
        metadata = {"backend": "embedded_cef", "app_version": version,
                    "cef_version": pin["cef_version"], "platform": platform,
                    "codec_build": "custom", "codec_capabilities": "not_tested",
                    "widevine_cdm_bundled": False, "widevine_vmp": "not_provided",
                    "signature_kind": "unsigned_development"}
        (payload / "runtime.json").write_text(json.dumps(metadata, indent=2) + "\n")
        (payload / "LEGGIMI.txt").write_text(
            f"Lume TV {version} — build desktop sperimentale\n\n{launch}\n\n"
            "Al primo utilizzo di una diretta protetta, il player può scaricare Widevine nel profilo dell’app.\n"
            "Le prove desktop di video, audio e DRM su Windows e Linux restano da completare.\n"
            "Licenze e avvisi sono nella cartella licenses.\n", encoding="utf-8")
    if any("widevine" in path.name.casefold() for path in payload.rglob("*")):
        raise ValueError("Refusing to package a Widevine component")
    return payload, executable


def zip_portable(payload: Path, output: Path) -> None:
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=6) as archive:
        for path in sorted(payload.rglob("*")):
            if path.is_file():
                archive.write(path, (Path(payload.name) / path.relative_to(payload)).as_posix())


def make_deb(payload: Path, output: Path, version: str, *, root: Path = ROOT) -> None:
    with tempfile.TemporaryDirectory(prefix=".lume-deb-", dir=payload.parent) as temporary:
        package = Path(temporary)
        shutil.copytree(payload, package / "opt/lume-tv")
        # dpkg-deb records root ownership without making the build user root.
        # This is the standard CEF sandbox helper, not --no-sandbox.
        (package / "opt/lume-tv/chrome-sandbox").chmod(0o4755)
        control = package / "DEBIAN"
        control.mkdir()
        (control / "control").write_text(
            f"Package: lume-tv\nVersion: {version}\nArchitecture: amd64\n"
            "Section: video\nPriority: optional\nMaintainer: Lume TV contributors\n"
            f"Depends: {', '.join(LINUX_DEPS)}, libc6 (>= 2.39)\n"
            "Description: Lume TV desktop experimental Rust/CEF player\n"
            " Italian live TV, catalogue and guide with an embedded player.\n")
        applications = package / "usr/share/applications"
        applications.mkdir(parents=True)
        (applications / "it.lumetv.desktop.desktop").write_text(
            "[Desktop Entry]\nType=Application\nName=Lume TV\nExec=/opt/lume-tv/AppRun\n"
            "Icon=lumetv\nTerminal=false\nCategories=AudioVideo;Video;\n")
        icons = package / "usr/share/pixmaps"
        icons.mkdir(parents=True)
        shutil.copy2(root / "web/app/assets/app-icon.png", icons / "lumetv.png")
        run(["dpkg-deb", "--root-owner-group", "--build", str(package), str(output)])
        contents = run(["dpkg-deb", "--contents", str(output)], capture=True)
        sandbox = next((line for line in contents.splitlines()
                        if line.rstrip().endswith("/opt/lume-tv/chrome-sandbox")), "")
        if not re.match(r"^-rwsr-xr-x\s+root/root\s", sandbox):
            raise ValueError("DEB is missing the root-owned CEF sandbox helper permissions")


def archive_package(platform: str, payload: Path, output: Path, *, root: Path = ROOT) -> list[Path]:
    version = app_version(root)
    stem = f"LumeTV-{version}-{platform}"
    if platform.startswith("macos-"):
        zipped = output / (stem + ".zip")
        run(["ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", str(payload), str(zipped)])
        # ditto preserves framework symlinks and executable permissions.
        with tempfile.TemporaryDirectory(prefix=".lume-dmg-", dir=payload.parent) as temporary:
            image_root = Path(temporary)
            shutil.copytree(payload, image_root / payload.name, symlinks=True)
            (image_root / "Applications").symlink_to("/Applications")
            dmg = output / (stem + ".dmg")
            run(["hdiutil", "create", "-volname", "Lume TV", "-srcfolder", str(image_root),
                 "-ov", "-format", "UDZO", str(dmg)])
        return [zipped, dmg]
    if platform == "windows-x64":
        zipped = output / (stem + ".zip")
        zip_portable(payload, zipped)
        return [zipped]
    compressed = output / (stem + ".tar.gz")
    with tarfile.open(compressed, "w:gz") as archive:
        archive.add(payload, arcname=payload.name)
    deb = output / (stem + ".deb")
    make_deb(payload, deb, version, root=root)
    return [compressed, deb]


def write_manifest(platform: str, artifacts: list[Path], *, root: Path = ROOT) -> Path:
    pin = cef_sdk.pins()[platform]
    version = app_version(root)
    manifest = artifacts[0].parent / f"LumeTV-{version}-{platform}.build.json"
    data = {"schema_version": 1, "version": version, "platform": platform,
            "source_revision": source_revision(root), "cef_version": pin["cef_version"],
            "sdk_sha256": pin["sha256"], "sandbox_enabled": True,
            "bindings_cef_version": json.loads(cef_sdk.PIN_FILE.read_text())["bindings_cef_version"],
            "cef_api_version": 999999,
            "widevine_cdm_bundled": False, "native_loader_smoke": "passed",
            "live_playback": "not_tested_by_pipeline", "distribution_signing": "development_only",
            "artifacts": [{"name": p.name, "size": p.stat().st_size, "sha256": cef_sdk.sha256(p)}
                          for p in artifacts]}
    manifest.write_text(json.dumps(data, indent=2) + "\n")
    checksums = manifest.with_suffix(".sha256")
    checksums.write_text("".join(f'{item["sha256"]}  {item["name"]}\n' for item in data["artifacts"])
                         + f"{cef_sdk.sha256(manifest)}  {manifest.name}\n")
    return manifest


def build(platform: str, *, toolchain="stable", output: Path | None = None,
          archive: Path | None = None) -> list[Path]:
    pin = cef_sdk.pins()[platform]
    native_host(pin)
    output = (output or ROOT / "dist").resolve()
    if not output.is_relative_to(ROOT):
        raise ValueError("Package output must be inside the project")
    output.mkdir(parents=True, exist_ok=True)
    sdk = cef_sdk.prepare(platform, archive=archive)
    target = ROOT / "target/desktop" / platform
    env = dict(os.environ, CEF_PATH=str(sdk), LUME_CEF_USE_PINNED_OBS="1",
               CARGO_TARGET_DIR=str(target), CMAKE_BUILD_PARALLEL_LEVEL="2", CARGO_BUILD_JOBS="2")
    if pin["os"] == "macos":
        env["MACOSX_DEPLOYMENT_TARGET"] = "13.0"
    if pin["os"] == "windows":
        env["RUSTFLAGS"] = env.get("RUSTFLAGS", "") + " -C target-feature=+crt-static"
    if pin["os"] == "linux":
        env["RUSTFLAGS"] = env.get("RUSTFLAGS", "") + " -C link-arg=-Wl,-rpath,$ORIGIN"
    cargo = ["cargo", "+" + toolchain]
    common = ["build", "--locked", "--release", "--target", pin["target"]]
    if pin["os"] == "windows":
        run(cargo + common + ["--bin", "lume-player"], env=env)
        run(cargo + common + ["-p", "lume-cef-windows"], env=env)
        run(cargo + ["clippy", "--locked", "--release", "--target", pin["target"],
                     "-p", "lume-cef-windows", "--", "-D", "warnings"], env=env)
    else:
        run(cargo + common + ["--features", "embedded-cef", "--bins"], env=env)
        run(cargo + ["clippy", "--locked", "--release", "--target", pin["target"],
                     "--features", "embedded-cef", "--bins", "--", "-D", "warnings"], env=env)
    binaries = target / pin["target"] / "release"
    target.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="package-", dir=target) as temporary:
        stage = Path(temporary)
        notices = stage / "notices"
        collect_notices(sdk, notices, cargo=cargo, env=env)
        payload, executable = assemble(platform, sdk, binaries, stage, notices)
        smoke_env = dict(env)
        if pin["os"] == "linux":
            smoke_env["LD_LIBRARY_PATH"] = str(payload)
        info = json.loads(subprocess.check_output([str(executable), "--lume-version"],
                                                 env=smoke_env, text=True, timeout=45))
        validate_runtime_info(info, pin)
        artifacts = archive_package(platform, payload, output)
    write_manifest(platform, artifacts)
    return artifacts


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", choices=cef_sdk.pins(), required=True)
    parser.add_argument("--toolchain", default="stable")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--archive", type=Path, help="Already downloaded official SDK")
    args = parser.parse_args()
    for artifact in build(args.platform, toolchain=args.toolchain, output=args.output, archive=args.archive):
        print(artifact)
