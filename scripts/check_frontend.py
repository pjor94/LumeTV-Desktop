#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Match the embedded React assets to their sources, without requiring Node."""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ASSETS = ("app.css", "app.js", "index.html")


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def fingerprint(root: Path = ROOT) -> dict:
    ui = root / "web/app"
    sources = [ui / name for name in ("index.html", "app.css", "motion.css", "vite.config.ts", "tsconfig.json", "package.json", "package-lock.json")]
    sources += [path for path in (ui / "src").rglob("*") if path.is_file()]
    sources += [root / "web/player.js"]
    return {
        "schema_version": 1,
        "sources": {path.relative_to(root).as_posix(): digest(path) for path in sorted(sources)},
        "assets": {name: digest(ui / "bundle" / name) for name in ASSETS},
    }


def check(root: Path = ROOT) -> None:
    directory = root / "web/app/bundle"
    stored = json.loads((directory / "manifest.json").read_text())
    if stored != fingerprint(root):
        raise ValueError("React sources and embedded assets differ; run npm run build in web/app")
    if {path.name for path in directory.iterdir()} != {*ASSETS, "manifest.json"}:
        raise ValueError("Unexpected frontend output; update the explicit Rust asset map")
    html = (directory / "index.html").read_text()
    for resource in ("/app.js", "/app.css", "/player.js", "/vendor/shaka-player.compiled.js"):
        if resource not in html:
            raise ValueError("Missing embedded frontend resource: " + resource)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stamp", action="store_true", help="Record fingerprints after a reviewed frontend build")
    args = parser.parse_args()
    try:
        if args.stamp:
            (ROOT / "web/app/bundle/manifest.json").write_text(json.dumps(fingerprint(), indent=2, ensure_ascii=False) + "\n")
        check()
    except (ValueError, OSError) as error:
        print(str(error))
        return 1
    print("Embedded React assets match their sources.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
