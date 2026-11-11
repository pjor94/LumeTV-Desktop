#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Tagged desktop releases: matching source, asset verification and publication.

Only the publish subcommand writes to GitHub. It requires GitHub Actions, an
existing matching tag, every native package, and the corresponding vendored
source archive from the same commit. No raw reports or profiles are uploaded.
"""
from __future__ import annotations

import argparse
import gzip
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile
import tomllib

import build_desktop
import check_frontend
import check_repository
import prepare_cef_sdk as cef_sdk
import update_feed

ROOT = Path(__file__).resolve().parents[1]
TAG = re.compile(r"v(\d+\.\d+\.\d+(?:-[A-Za-z0-9.-]+)?)\Z")


def validate_tag(tag: str, version: str) -> None:
    match = TAG.fullmatch(tag)
    if not match or match[1] != version:
        raise ValueError("Tag must be v followed by the exact Cargo package version")


def source_metadata(tag: str, *, root: Path = ROOT) -> dict:
    version = build_desktop.app_version(root)
    validate_tag(tag, version)
    windows = tomllib.loads((root / "crates/lume-cef-windows/Cargo.toml").read_text())
    if windows["package"]["version"] != version:
        raise ValueError("Windows DLL and application versions differ")
    ui = root / "web/app"
    package = json.loads((ui / "package.json").read_text())
    lock = json.loads((ui / "package-lock.json").read_text())
    if any(item != version for item in (package["version"], lock["version"], lock["packages"][""]["version"])):
        raise ValueError("Frontend and application versions differ")
    cargo = tomllib.loads((root / "Cargo.lock").read_text())
    packages = [p for p in cargo["package"] if p["name"] in {"lume-player", "lume-cef-windows"}]
    if len(packages) != 2 or any(p["version"] != version for p in packages):
        raise ValueError("Cargo lockfile and application versions differ")
    changelog_notes(version, root=root)
    revision = build_desktop.source_revision(root)
    if not re.fullmatch(r"[0-9a-f]{40,64}", revision):
        raise ValueError("Invalid source commit")
    return {"tag": tag, "version": version, "revision": revision,
            "prerelease": "-" in version}


def release_metadata(tag: str, *, root: Path = ROOT) -> dict:
    metadata = source_metadata(tag, root=root)
    tagged = subprocess.check_output(["git", "rev-parse", "--verify", f"refs/tags/{tag}^{{commit}}"],
                                     cwd=root, text=True).strip()
    if tagged != metadata["revision"]:
        raise ValueError("Checked-out commit differs from the existing version tag")
    return metadata


def changelog_notes(version: str, *, root: Path = ROOT) -> str:
    section = re.search(r"^## " + re.escape(version) + r"(?:[ \t][^\n]*)?\n(.*?)(?=^## |\Z)",
                        (root / "CHANGELOG.md").read_text(), re.MULTILINE | re.DOTALL)
    if not section or not section[1].strip():
        raise ValueError("Release version absent from changelog or notes empty")
    return section[1].strip()


def canonical_tar(info: tarfile.TarInfo) -> tarfile.TarInfo:
    info.uid = info.gid = 0
    info.uname = info.gname = ""
    info.mtime = 0
    info.mode &= 0o777
    info.pax_headers = {}
    return info


def archive_source(snapshot: Path, output: Path) -> None:
    with output.open("wb") as raw, gzip.GzipFile(filename="", fileobj=raw, mode="wb", mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
            archive.add(snapshot, arcname=snapshot.name, filter=canonical_tar)


def package_source(*, root: Path = ROOT, output: Path | None = None,
                   cargo: list[str] | None = None) -> Path:
    version = build_desktop.app_version(root)
    if not check_repository.index_matches_worktree(root):
        raise ValueError("Source packaging requires reviewed files matching the Git index")
    names = check_repository.git_files(root, tracked_only=True)
    findings = check_repository.audit_files(root, names)
    findings.extend("Missing required source file: " + name
                    for name in sorted(check_repository.REQUIRED_FILES - set(names)))
    if findings:
        raise ValueError("Source snapshot failed repository checks: " + "; ".join(findings))
    lock = tomllib.loads((root / "Cargo.lock").read_text())
    if any(p.get("source") and p["source"] != "registry+https://github.com/rust-lang/crates.io-index"
           for p in lock["package"]):
        raise ValueError("New registry or Git dependencies require explicit source vendoring support")
    output = (output or root / "dist").resolve()
    if not output.is_relative_to(root.resolve()):
        raise ValueError("Source package output must be inside the project")
    output.mkdir(parents=True, exist_ok=True)
    cache = root / ".cache/release-source"
    cache.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="source-", dir=cache) as temporary:
        work = Path(temporary)
        dependencies = work / "cargo"
        subprocess.run((cargo or ["cargo"]) + ["vendor", "--locked", "--versioned-dirs", str(dependencies)],
                       cwd=root, check=True, stdout=subprocess.PIPE, text=True)
        snapshot = work / f"LumeTV-{version}-source"
        snapshot.mkdir()
        for name in names:
            destination = snapshot / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(root / name, destination)
        configuration = snapshot / ".cargo/config.toml"
        if configuration.exists() or (snapshot / "third_party/cargo").exists():
            raise ValueError("Source vendoring would overwrite a tracked Cargo configuration")
        for path in dependencies.rglob("*"):
            if path.is_symlink():
                raise ValueError("Unexpected symlink in Cargo dependency sources")
        shutil.copytree(dependencies, snapshot / "third_party/cargo")
        configuration.parent.mkdir(exist_ok=True)
        configuration.write_text(
            '[source.crates-io]\nreplace-with = "vendored-sources"\n\n'
            '[source.vendored-sources]\ndirectory = "third_party/cargo"\n')
        (snapshot / "VENDORED-SOURCE.md").write_text(
            "# Sorgenti corrispondenti\n\n"
            "Questo archivio contiene il checkout e i sorgenti delle dipendenze Cargo risolte nel lockfile.\n"
            "Il core si puo' compilare con `cargo build --locked --offline`.\n"
            "Per il player incorporato seguire docs/RELEASE.md: l'SDK CEF e' un download separato, verificato con i pin inclusi.\n"
            "Il CDM Widevine non e' incluso.\n")
        archive = output / f"LumeTV-{version}-source.tar.gz"
        archive_source(snapshot, archive)
        packages = len(list(dependencies.iterdir()))
    manifest = output / f"LumeTV-{version}-source.build.json"
    manifest.write_text(json.dumps({
        "schema_version": 1, "platform": "source", "version": version,
        "source_revision": build_desktop.source_revision(root), "vendored_cargo_packages": packages,
        "artifacts": [{"name": archive.name, "size": archive.stat().st_size, "sha256": cef_sdk.sha256(archive)}],
    }, indent=2) + "\n")
    manifest.with_suffix(".sha256").write_text(
        f"{cef_sdk.sha256(archive)}  {archive.name}\n{cef_sdk.sha256(manifest)}  {manifest.name}\n")
    return archive


def verify_assets(directory: Path, version: str, revision: str) -> list[Path]:
    if any(path.is_symlink() for path in directory.iterdir()):
        raise ValueError("Release output must not contain symlinks")
    expected = set(cef_sdk.pins()) | {"source"}
    found = set()
    assets = []
    for manifest in sorted(directory.glob("*.build.json")):
        data = json.loads(manifest.read_text())
        name = data["platform"]
        if name not in expected or name in found:
            raise ValueError("Duplicate or unknown artifact platform")
        found.add(name)
        if data["version"] != version or data["source_revision"] != revision:
            raise ValueError("Artifact version/commit differs from the tagged source")
        if manifest.name != f"LumeTV-{version}-{name}.build.json":
            raise ValueError("Artifact manifest name mismatch")
        if name != "source":
            pin = cef_sdk.pins()[name]
            if (data.get("sdk_sha256") != pin["sha256"] or data.get("cef_version") != pin["cef_version"]
                    or data.get("bindings_cef_version") != json.loads(cef_sdk.PIN_FILE.read_text())["bindings_cef_version"]
                    or data.get("cef_api_version") != 999999
                    or data.get("widevine_cdm_bundled") is not False
                    or data.get("sandbox_enabled") is not True
                    or data.get("native_loader_smoke") != "passed"):
                raise ValueError("Runtime provenance, loader smoke or sandbox check missing")
        elif data.get("vendored_cargo_packages", 0) < 1:
            raise ValueError("Matching Cargo dependency sources are missing")
        recorded_names = set()
        for artifact in data["artifacts"]:
            filename = artifact["name"]
            if (Path(filename).name != filename or filename in recorded_names
                    or not filename.startswith(f"LumeTV-{version}-{name}.")
                    or not re.fullmatch(r"[0-9a-f]{64}", artifact["sha256"])):
                raise ValueError("Invalid artifact name or checksum")
            recorded_names.add(filename)
            path = directory / filename
            if path.is_symlink() or not path.is_file() or path.stat().st_size != artifact["size"]:
                raise ValueError("Downloaded artifact missing or size differs")
            if cef_sdk.sha256(path) != artifact["sha256"]:
                raise ValueError("Downloaded artifact checksum differs")
            assets.append(path)
        suffixes = {p.name[len(f"LumeTV-{version}-{name}"):] for p in assets
                    if p.name.startswith(f"LumeTV-{version}-{name}")}
        required = ({".dmg", ".zip"} if name.startswith("macos-") else
                    {".zip"} if name == "windows-x64" else
                    {".tar.gz", ".deb"} if name == "linux-x64" else {".tar.gz"})
        if suffixes != required:
            raise ValueError("Missing or unexpected package format for " + name)
        checksum_file = manifest.with_suffix(".sha256")
        expected_lines = [f'{item["sha256"]}  {item["name"]}' for item in data["artifacts"]]
        expected_lines.append(f"{cef_sdk.sha256(manifest)}  {manifest.name}")
        if not checksum_file.is_file() or checksum_file.read_text().splitlines() != expected_lines:
            raise ValueError("Downloaded checksum sidecar differs")
        assets.extend([manifest, checksum_file])
    if found != expected:
        raise ValueError("All four native builds and matching sources are required")
    allowed = {path.name for path in assets} | {"SHA256SUMS.txt", "RELEASE_NOTES.md"}
    if any(path.name not in allowed or not path.is_file() for path in directory.iterdir()):
        raise ValueError("Unexpected file in release output; refusing broad upload")
    return sorted(assets)


def render_notes(metadata: dict, *, root: Path = ROOT) -> str:
    version = metadata["version"]
    base = f"https://github.com/pjor94/LumeTV-Desktop/releases/download/v{version}/LumeTV-{version}"
    channel = "Versione di anteprima.\n\n" if metadata["prerelease"] else ""
    return (
        f'# Lume TV Desktop {metadata["tag"]}\n\n{channel}'
        "## Novità\n\n" + changelog_notes(version, root=root) + "\n\n"
        "## Download\n\n"
        "| Sistema | Pacchetti |\n| --- | --- |\n"
        f"| macOS 13+ Apple Silicon | [DMG]({base}-macos-arm64.dmg) · [ZIP]({base}-macos-arm64.zip) |\n"
        f"| macOS 13+ Intel | [DMG]({base}-macos-x64.dmg) · [ZIP]({base}-macos-x64.zip) |\n"
        f"| Windows x64 | [ZIP portabile]({base}-windows-x64.zip) |\n"
        f"| Linux x64 / Ubuntu 24.04 | [DEB]({base}-linux-x64.deb) · [tar.gz]({base}-linux-x64.tar.gz) |\n\n"
        "Su macOS copia Lume TV.app in Applicazioni. Su Windows estrai tutta la cartella "
        "e avvia LumeTV.exe, lasciando DLL e risorse al loro posto. Il DEB installa "
        "le dipendenze e l’helper sandbox; il tar.gz richiede user namespace disponibili.\n\n"
        "## Passaggio dalla 0.1.0\n\n"
        "La 0.1.0 non ha l’updater: chiudi Lume e installa questa versione dal pacchetto. "
        "Preferiti e impostazioni restano nella cartella dati dell’utente. Dalla 0.2.0 "
        "puoi controllare e scaricare le nuove versioni nelle Impostazioni dell’app.\n\n"
        "Al primo avvio Lume prepara Widevine prima di aprire il catalogo. "
        "Serve Internet e possono volerci alcuni minuti. Chi ha già un componente "
        "utilizzabile passa direttamente al catalogo. Il player usa CEF incorporato: "
        "non serve installare Chrome.\n\n"
        "## Verifiche e limiti\n\n"
        "macOS usa una firma ad hoc senza notarizzazione; Windows non ha una firma "
        "Authenticode. La firma Ed25519 dell’updater verifica la provenienza dei pacchetti "
        "e rimane distinta dai controlli del sistema operativo.\n\n"
        "Le prove di riproduzione disponibili riguardano macOS Apple Silicon. "
        "Immagine sul monitor e ascolto dei canali protetti, aggiornamenti fra release "
        "complete e collaudo desktop degli altri sistemi restano da completare. "
        "Lo stato dettagliato delle prove è in "
        "[STATUS.md](https://github.com/pjor94/LumeTV-Desktop/blob/" + metadata["revision"] + "/docs/STATUS.md).\n\n"
        f'Pacchetti e [sorgenti]({base}-source.tar.gz) corrispondono al commit `{metadata["revision"]}`. '
        "I sorgenti includono dipendenze Cargo e licenze. SHA256SUMS.txt contiene "
        "le impronte degli allegati; update.json e update.sig sono usati dall’app.\n"
    )


def prepare(tag: str, directory: Path, *, root: Path = ROOT) -> Path:
    metadata = source_metadata(tag, root=root)
    if subprocess.check_output(["git", "status", "--porcelain"], cwd=root, text=True).strip():
        raise ValueError("Local release preparation requires committed, reviewed source")
    check_frontend.check(root)
    names = check_repository.git_files(root, tracked_only=True)
    findings = check_repository.audit_files(root, names)
    findings += ["Missing required source file: " + name for name in sorted(check_repository.REQUIRED_FILES - set(names))]
    if findings:
        raise ValueError("Source snapshot failed repository checks: " + "; ".join(findings))
    directory = directory.resolve()
    if not directory.is_relative_to(root.resolve()):
        raise ValueError("Local release output must be inside the project")
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "RELEASE_NOTES.md").write_text(render_notes(metadata, root=root), encoding="utf-8")
    plan = {**metadata, "local_only": True, "release_created": False,
            "native_targets": list(cef_sdk.pins()), "required_secret": "LUME_UPDATE_SIGNING_KEY",
            "publication": "Push the reviewed commit and its version tag only when ready to publish."}
    (directory / "release-plan.json").write_text(json.dumps(plan, indent=2) + "\n")
    return directory


def collect(tag: str, directory: Path, *, root: Path = ROOT) -> list[Path]:
    metadata = release_metadata(tag, root=root)
    assets = verify_assets(directory, metadata["version"], metadata["revision"])
    checksums = directory / "SHA256SUMS.txt"
    checksums.write_text("".join(f"{cef_sdk.sha256(p)}  {p.name}\n" for p in assets))
    (directory / "RELEASE_NOTES.md").write_text(render_notes(metadata, root=root), encoding="utf-8")
    return assets + [checksums]


def publication_command(tag: str, directory: Path, assets: list[Path]) -> list[str]:
    version = tag.removeprefix("v")
    validate_tag(tag, version)
    channel = ["--prerelease", "--latest=false"] if "-" in version else ["--latest"]
    return ["gh", "release", "create", tag, "--repo", "pjor94/LumeTV-Desktop", "--verify-tag", *channel,
            "--title", "Lume TV Desktop " + tag, "--notes-file", str(directory / "RELEASE_NOTES.md"),
            *map(str, assets)]


def publish(tag: str, directory: Path, *, root: Path = ROOT) -> None:
    if os.environ.get("GITHUB_ACTIONS") != "true" or not os.environ.get("GH_TOKEN"):
        raise ValueError("Publication is reserved for the authenticated GitHub release job")
    assets = collect(tag, directory, root=root)
    private_key = os.environ.get("LUME_UPDATE_SIGNING_KEY")
    if not private_key:
        raise ValueError("The release requires LUME_UPDATE_SIGNING_KEY")
    version = tag.removeprefix("v")
    notes = changelog_notes(version, root=root)
    manifest = update_feed.build_manifest(directory, version, "-" in version, notes)
    assets += update_feed.sign_manifest(manifest, directory, private_key)
    checksums = directory / "SHA256SUMS.txt"
    checksums.write_text("".join(f"{cef_sdk.sha256(p)}  {p.name}\n" for p in assets if p != checksums))
    # No --clobber: an existing release must be reviewed instead of overwritten.
    subprocess.run(publication_command(tag, directory, assets), cwd=root, check=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("prepare", "metadata", "source", "collect", "publish"))
    parser.add_argument("--tag", default=os.environ.get("RELEASE_TAG", ""))
    parser.add_argument("--directory", type=Path)
    args = parser.parse_args()
    directory = args.directory or (ROOT / ".cache/release" if args.command == "prepare" else ROOT / "dist")
    if args.command == "prepare":
        print(prepare(args.tag, directory))
    elif args.command == "metadata":
        data = release_metadata(args.tag)
        if os.environ.get("GITHUB_OUTPUT"):
            with Path(os.environ["GITHUB_OUTPUT"]).open("a") as output:
                output.write("".join(f"{key}={str(value).lower() if isinstance(value, bool) else value}\n" for key, value in data.items()))
        print(json.dumps(data))
    elif args.command == "source":
        print(package_source(output=directory))
    elif args.command == "collect":
        print(json.dumps([path.name for path in collect(args.tag, directory)]))
    else:
        publish(args.tag, directory)
