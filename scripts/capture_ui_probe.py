#!/usr/bin/env python3
"""Capture sanitized metrics from an explicitly selected local CEF test profile."""
import argparse
import json
from pathlib import Path
import re
import subprocess
import urllib.request


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    profile = str((args.profile / "browser").resolve())
    commands = subprocess.check_output(["ps", "-axo", "command"], text=True)
    lines = [line for line in commands.splitlines()
             if "Contents/MacOS/lume-cef --lume-url=" in line
             and f"--lume-profile={profile}" in line]
    if len(lines) != 1:
        raise SystemExit("Expected one running CEF window for the selected profile")
    match = re.search(r"--lume-url=(http://127\.0\.0\.1:\d+/#[-\w]+)", lines[0])
    if not match:
        raise SystemExit("Local test endpoint not found")
    origin, token = match.group(1).split("/#")

    def get(path):
        request = urllib.request.Request(origin + path, headers={"X-Lume-Token": token})
        with urllib.request.urlopen(request, timeout=10) as response:
            return json.load(response)

    report = get("/api/playback/status")
    catalog = get("/api/catalog")
    guide = get("/api/guide")
    report["ui"] = {
        "catalog_channels": len(catalog["channels"]),
        "catalog_updated_at": catalog["updated_at"],
        "catalog_loading": catalog["loading"],
        "catalog_error": catalog["error"],
        "guide_channels": len(guide["programmes"]),
        "guide_updated_at": guide["updated_at"],
    }
    encoded = json.dumps(report, indent=2)
    if re.search(r"https?://", encoded) or any(key in encoded for key in ('"license_url"', '"headers"', '"token"')):
        raise SystemExit("Refusing to write a report containing private source metadata")
    args.output.write_text(encoded + "\n")
    assessment = report.get("assessment", {})
    print(json.dumps({"channel_id": report.get("channel_id"), "status": assessment.get("status"),
                      "progress_seconds": assessment.get("progress_seconds"),
                      "drm_verified": assessment.get("drm_verified"), "report": str(args.output)}))


if __name__ == "__main__":
    main()
