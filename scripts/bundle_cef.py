#!/usr/bin/env python3
"""Create a local development CEF bundle without copying a Widevine CDM."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import tempfile
import tomllib
import uuid


APP_NAME = "lume-cef"
HELPERS = ("Helper (GPU)", "Helper (Renderer)", "Helper (Plugin)", "Helper (Alerts)", "Helper")
FRAMEWORK = "Chromium Embedded Framework.framework"
MARKER = Path("Contents/Resources/.lume-cef-generated")
MARKER_CONTENT = "lume-cef-development-bundle-v1\n"
PROJECT_ROOT = Path(__file__).resolve().parents[1]


def read_versions(header: Path) -> dict[str, str]:
    text = header.read_text(encoding="utf-8")
    version = re.search(r'^\s*#define\s+CEF_VERSION\s+"([^"\n]+)"', text, re.MULTILINE)
    if version is None:
        raise ValueError("The SDK cef_version.h does not define CEF_VERSION")
    parts = []
    for part in ("MAJOR", "MINOR", "BUILD", "PATCH"):
        match = re.search(rf"^\s*#define\s+CHROME_VERSION_{part}\s+(\d+)", text, re.MULTILINE)
        if match is None:
            raise ValueError("The SDK cef_version.h is missing Chromium version fields")
        parts.append(match.group(1))
    return {"cef_version": version.group(1), "chromium_version": ".".join(parts)}


def find_sdk_files(sdk: Path) -> tuple[Path, Path]:
    for root in (sdk, sdk / "Release"):
        framework = root / FRAMEWORK
        if framework.is_dir():
            break
    else:
        raise ValueError("The SDK does not contain the CEF framework")
    for header in (sdk / "include/cef_version.h", sdk.parent / "include/cef_version.h"):
        if header.is_file():
            return framework, header
    raise ValueError("The SDK does not contain include/cef_version.h")


def app_layout(path: Path, executable: str, identifier: str, *, helper: bool, version: str = "0.1.0") -> Path:
    macos = path / "Contents/MacOS"
    macos.mkdir(parents=True)
    (path / "Contents/Resources").mkdir()
    info = {
        "CFBundleName": executable if helper else "Lume TV",
        "CFBundleDisplayName": executable if helper else "Lume TV",
        "CFBundleExecutable": executable,
        "CFBundleIdentifier": identifier,
        "CFBundleInfoDictionaryVersion": "6.0",
        "CFBundlePackageType": "APPL",
        "CFBundleVersion": version.split("-", 1)[0].split("+", 1)[0],
        "CFBundleShortVersionString": version.split("-", 1)[0].split("+", 1)[0],
        "LSMinimumSystemVersion": "13.0",
        "LSUIElement": helper,
        "NSHighResolutionCapable": True,
        "NSSupportsAutomaticGraphicsSwitching": True,
        "LSEnvironment": {"MallocNanoZone": "0"},
    }
    with (path / "Contents/Info.plist").open("wb") as stream:
        plistlib.dump(info, stream)
    return macos / executable


def ignore_cdm(_directory: str, names: list[str]) -> set[str]:
    # The SDK's ordinary browser resources stay intact; proprietary CDMs never
    # become a bundled dependency, including in a custom SDK supplied by a user.
    return {name for name in names if "widevine" in name.casefold()}


def is_macho(path: Path) -> bool:
    if path.is_symlink() or not path.is_file():
        return False
    with path.open("rb") as stream:
        return stream.read(4) in {
            b"\xfe\xed\xfa\xce", b"\xce\xfa\xed\xfe",
            b"\xfe\xed\xfa\xcf", b"\xcf\xfa\xed\xfe",
            b"\xca\xfe\xba\xbe", b"\xbe\xba\xfe\xca",
            b"\xca\xfe\xba\xbf", b"\xbf\xba\xfe\xca",
        }


def codesign_bundle(app: Path, helpers: list[Path], framework: Path) -> str:
    executable = shutil.which("codesign")
    if executable is None:
        raise ValueError("macOS codesign is required; --skip-sign is only for fixtures/inspection")

    def sign(path: Path, entitlements: Path | None = None) -> None:
        command = [executable, "--force", "--sign", "-", "--timestamp=none"]
        if entitlements is not None:
            command += ["--entitlements", str(entitlements)]
        command.append(str(path))
        result = subprocess.run(command, capture_output=True, check=False)
        if result.returncode:
            detail = result.stderr.decode("utf-8", errors="replace").strip()[:800]
            raise ValueError(
                f"Ad-hoc signing failed for {path.name} (exit {result.returncode}): {detail}"
            )

    valid_framework = subprocess.run(
        [executable, "--verify", "--deep", "--strict", str(framework)],
        capture_output=True, check=False,
    ).returncode == 0
    framework_signature = "vendor_preserved" if valid_framework else "local_ad_hoc"
    if not valid_framework:
        for binary in sorted(framework.rglob("*"), key=lambda p: len(p.parts), reverse=True):
            if binary.name != "Chromium Embedded Framework" and is_macho(binary):
                sign(binary)
        sign(framework)

    # This is ordinary local code signing, not Widevine/VMP certification.
    # No hardened-runtime restriction or browser sandbox is disabled here.
    with tempfile.TemporaryDirectory(prefix="lume-cef-sign-") as temp:
        entitlements = Path(temp) / "jit.plist"
        with entitlements.open("wb") as stream:
            plistlib.dump({"com.apple.security.cs.allow-jit": True}, stream)
        for helper in helpers:
            sign(helper, entitlements)
        sign(app / "Contents/MacOS/lume-updater")
        sign(app, entitlements)

    verification = subprocess.run(
        [executable, "--verify", "--deep", "--strict", str(app)],
        capture_output=True, check=False,
    )
    if verification.returncode:
        detail = verification.stderr.decode("utf-8", errors="replace").strip()[:800]
        raise ValueError(
            f"Codesign verification failed for {app.name} "
            f"(exit {verification.returncode}): {detail}"
        )
    return framework_signature


def build_bundle(
    sdk: Path,
    target_dir: Path,
    output: Path,
    *,
    codec_build: str = "stock",
    skip_sign: bool = False,
    project_root: Path = PROJECT_ROOT,
    version: str | None = None,
    additional_licenses: Path | None = None,
) -> Path:
    project_root = project_root.resolve()
    if version is None:
        manifest = project_root / "Cargo.toml"
        version = tomllib.loads(manifest.read_text())["package"]["version"] if manifest.is_file() else "0.1.0"
    if not re.fullmatch(r"\d+\.\d+\.\d+(?:-[A-Za-z0-9.-]+)?", version):
        raise ValueError("Invalid application version")
    sdk = sdk.resolve()
    target_dir = target_dir.resolve()
    if output.is_symlink():
        raise ValueError("The bundle output must not be a symlink")
    output = output.resolve()
    if output.name not in {"lume-cef.app", "Lume TV.app"} or not output.is_relative_to(project_root):
        raise ValueError("The output must be a lume-cef.app or Lume TV.app bundle inside this project")
    if output.exists():
        marker = output / MARKER
        if not marker.is_file() or marker.read_text() != MARKER_CONTENT:
            raise ValueError("Refusing to replace an application not generated by this script")
    if codec_build not in {"stock", "custom"}:
        raise ValueError("codec_build must be stock or custom")
    main_binary = target_dir / APP_NAME
    helper_binary = target_dir / "lume-cef-helper"
    updater_binary = target_dir / "lume-updater"
    if not all(path.is_file() for path in (main_binary, helper_binary, updater_binary)):
        raise ValueError("Build lume-cef, lume-cef-helper and lume-updater before creating the bundle")
    framework_source, header = find_sdk_files(sdk)
    versions = read_versions(header)
    output.parent.mkdir(parents=True, exist_ok=True)
    stage = output.parent / f".lume-cef-build-{uuid.uuid4().hex}.app"
    backup = None
    try:
        executable = app_layout(stage, APP_NAME, "it.lumetv.desktop", helper=False, version=version)
        shutil.copy2(main_binary, executable)
        shutil.copy2(updater_binary, stage / "Contents/MacOS/lume-updater")
        (stage / "Contents/MacOS/lume-updater").chmod(0o755)
        executable.chmod(executable.stat().st_mode | 0o111)
        icon = project_root / "web/app/assets/LumeTV.icns"
        if icon.is_file():
            shutil.copy2(icon, stage / "Contents/Resources/LumeTV.icns")
            info_path = stage / "Contents/Info.plist"
            info = plistlib.loads(info_path.read_bytes())
            info["CFBundleIconFile"] = "LumeTV.icns"
            info_path.write_bytes(plistlib.dumps(info))
        frameworks = stage / "Contents/Frameworks"
        frameworks.mkdir()
        framework = frameworks / FRAMEWORK
        shutil.copytree(framework_source, framework, symlinks=True, ignore=ignore_cdm)
        helpers = []
        for index, suffix in enumerate(HELPERS):
            name = f"{APP_NAME} {suffix}"
            helper_app = frameworks / f"{name}.app"
            helper_executable = app_layout(
                helper_app, name, f"it.lumetv.desktop.helper{index}", helper=True, version=version
            )
            shutil.copy2(helper_binary, helper_executable)
            helper_executable.chmod(helper_executable.stat().st_mode | 0o111)
            helpers.append(helper_app)

        licenses = stage / "Contents/Resources/licenses"
        licenses.mkdir()
        frontend_licenses = project_root / "web/app/licenses"
        if frontend_licenses.is_dir():
            shutil.copytree(frontend_licenses, licenses / "frontend")
        if additional_licenses is not None:
            shutil.copytree(additional_licenses, licenses / "dependencies")
        for name in ("LICENSE", "THIRD_PARTY_NOTICES.md"):
            source = project_root / name
            if source.is_file():
                shutil.copy2(source, licenses / name)
        shutil.copy2(header, licenses / "cef_version.h")
        for name in ("LICENSE", "LICENSE.txt", "CREDITS.html"):
            for root in (sdk, sdk.parent):
                source = root / name
                if source.is_file():
                    shutil.copy2(source, licenses / f"CEF-{name}")
                    break

        metadata = {
            "backend": "embedded_cef",
            "app_version": version,
            **versions,
            "codec_build": codec_build,
            "codec_capabilities": "not_tested",
            "widevine_cdm_bundled": False,
            "widevine_vmp": "not_provided",
            "signature_kind": "unsigned_fixture" if skip_sign else "ad_hoc_development",
        }
        (stage / "Contents/Resources/runtime.json").write_text(
            json.dumps(metadata, indent=2) + "\n", encoding="utf-8"
        )
        (stage / MARKER).write_text(MARKER_CONTENT, encoding="utf-8")
        if not skip_sign:
            codesign_bundle(stage, helpers, framework)

        # Complete the new bundle before replacing an earlier generated copy.
        if output.exists():
            backup = output.parent / f".lume-cef-previous-{uuid.uuid4().hex}.app"
            output.rename(backup)
        try:
            stage.rename(output)
        except OSError:
            if backup is not None:
                backup.rename(output)
                backup = None
            raise
        if backup is not None:
            shutil.rmtree(backup)
        return output
    finally:
        if stage.exists():
            shutil.rmtree(stage)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk", required=True, type=Path)
    parser.add_argument("--target-dir", type=Path, default=Path("target/cef-arm64/debug"))
    parser.add_argument("--output", type=Path, default=Path("target/cef-arm64/debug/lume-cef.app"))
    parser.add_argument("--codec-build", choices=("stock", "custom"), default="stock")
    parser.add_argument("--skip-sign", action="store_true", help="Fixtures/inspection only")
    args = parser.parse_args()
    try:
        output = build_bundle(
            args.sdk, args.target_dir, args.output,
            codec_build=args.codec_build, skip_sign=args.skip_sign,
        )
    except (ValueError, OSError) as error:
        parser.exit(1, f"Bundle preparation failed: {error}\n")
    print(output)


if __name__ == "__main__":
    main()
