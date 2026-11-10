# SPDX-License-Identifier: GPL-3.0-or-later
"""Synthetic component fixtures; these tests never read or copy a real CDM."""

import contextlib
import importlib.util
import io
import json
from pathlib import Path
import struct
import sys
import tempfile
import unittest
from unittest import mock

SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "seed_widevine.py"
SPEC = importlib.util.spec_from_file_location("lume_seed_widevine", SCRIPT)
seed = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = seed
SPEC.loader.exec_module(seed)


class SeedWidevineTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name).resolve()
        self.source = self.root / "external-component"
        self.profile = self.root / "profile"
        self.source.mkdir()
        self.profile.mkdir()
        (self.profile / seed.PROFILE_MARKER).write_text("Lume dedicated profile\n")
        self.manifest = {
            "manifest_version": 2,
            "name": "WidevineCdm",
            "version": "4.10.3112.0",
            "x-cdm-module-versions": "4",
            "x-cdm-interface-versions": "10",
            "x-cdm-host-versions": "10,11",
            "platforms": [{"os": "mac", "arch": "arm64", "sub_package_path": "_platform_specific/mac_arm64/"}],
        }
        self.write_manifest()
        self.binary = self.source / "_platform_specific" / "mac_arm64" / "libwidevinecdm.dylib"
        self.binary.parent.mkdir(parents=True)
        self.binary.write_bytes(b"\xcf\xfa\xed\xfe" + struct.pack("<I", 0x0100000C) + b"synthetic-not-loadable")
        self.binary.with_suffix(".dylib.sig").write_bytes(b"synthetic-signature")
        (self.source / "LICENSE").write_text("Synthetic fixture only\n")

    def tearDown(self):
        self.temporary.cleanup()

    def write_manifest(self):
        (self.source / "manifest.json").write_text(json.dumps(self.manifest))

    def plan(self):
        return seed.prepare(self.source, self.profile, system="Darwin", machine="arm64")

    def test_check_is_read_only_and_actual_seed_preserves_source_and_hashes(self):
        before = {path.relative_to(self.source): path.read_bytes() for path in self.source.rglob("*") if path.is_file()}
        plan = self.plan()
        checked = seed.execute(plan, check=True)
        self.assertEqual(checked["status"], "ready_to_seed")
        self.assertFalse((self.profile / "WidevineCdm").exists())
        self.assertFalse(checked["cdm_loaded"])
        result = seed.execute(plan)
        self.assertEqual(result["status"], "seeded")
        self.assertEqual(result["platform"], "mac_arm64")
        self.assertFalse(result["drm_verified"])
        self.assertFalse(result["host_verification_signatures_provided"])
        self.assertTrue(result["source_external"])
        for relative, payload in before.items():
            self.assertEqual((self.source / relative).read_bytes(), payload)
            self.assertEqual((plan.destination / relative).read_bytes(), payload)
        self.assertFalse((plan.destination.parent / seed.HINT_NAME).exists())
        self.assertEqual(seed.execute(plan)["status"], "already_seeded")
        serialized = json.dumps(result)
        self.assertNotIn("synthetic-signature", serialized)
        self.assertNotIn(str(self.source), serialized)
        self.assertNotIn(str(self.profile), serialized)

    def test_marker_and_nonoverlapping_absolute_paths_are_required(self):
        (self.profile / seed.PROFILE_MARKER).unlink()
        with self.assertRaisesRegex(seed.SeedError, "dedicated_profile_marker_required"):
            self.plan()
        (self.source / seed.PROFILE_MARKER).write_text("marker")
        with self.assertRaisesRegex(seed.SeedError, "source_profile_overlap"):
            seed.prepare(self.source, self.source, system="Darwin", machine="arm64")
        with self.assertRaisesRegex(seed.SeedError, "source_must_be_absolute"):
            seed.prepare(Path("relative"), self.profile, system="Darwin", machine="arm64")

    def test_version_cannot_escape_destination_and_architecture_is_checked(self):
        for version in ["../escape", "4.10.3112/0", "not-a-version", "4.10.4294967296.0"]:
            self.manifest["version"] = version
            self.write_manifest()
            with self.assertRaisesRegex(seed.SeedError, "invalid_component_version"):
                self.plan()
        self.manifest["version"] = "4.10.3112.0"
        self.write_manifest()
        self.binary.write_bytes(b"\xcf\xfa\xed\xfe" + struct.pack("<I", 0x01000007))
        with self.assertRaisesRegex(seed.SeedError, "library_architecture_mismatch"):
            self.plan()

    def test_manifest_declared_platform_must_match_the_current_runtime(self):
        self.manifest["platforms"][0]["arch"] = "x64"
        self.write_manifest()
        with self.assertRaisesRegex(seed.SeedError, "manifest_platform_mismatch"):
            self.plan()

    def test_symlink_unknown_file_and_component_size_are_rejected(self):
        payload = self.binary.read_bytes()
        self.binary.unlink()
        target = self.root / "outside-library"
        target.write_bytes(payload)
        self.binary.symlink_to(target)
        with self.assertRaisesRegex(seed.SeedError, "component_symlink_rejected"):
            self.plan()
        self.binary.unlink()
        self.binary.write_bytes(payload)
        extra = self.binary.parent / "unrelated-browser-binary"
        extra.write_bytes(b"unrelated")
        with self.assertRaisesRegex(seed.SeedError, "unexpected_platform_file"):
            self.plan()
        extra.unlink()
        with mock.patch.object(seed, "MAX_COMPONENT_BYTES", 16):
            with self.assertRaises(seed.SeedError):
                self.plan()

    def test_existing_component_is_never_overwritten_when_bytes_differ(self):
        plan = self.plan()
        seed.execute(plan)
        target = plan.destination / self.binary.relative_to(self.source)
        target.write_bytes(b"preserve-existing-component")
        with self.assertRaisesRegex(seed.SeedError, "existing_component_conflict"):
            seed.execute(plan)
        self.assertEqual(target.read_bytes(), b"preserve-existing-component")

    def test_linux_hint_uses_version_directory_and_does_not_invent_bundled_version(self):
        self.manifest.pop("platforms")
        self.write_manifest()
        linux_binary = self.source / "_platform_specific" / "linux_x64" / "libwidevinecdm.so"
        linux_binary.parent.mkdir()
        elf = bytearray(64)
        elf[:6] = b"\x7fELF\x02\x01"
        struct.pack_into("<H", elf, 18, 62)
        linux_binary.write_bytes(elf)
        plan = seed.prepare(self.source, self.profile, system="Linux", machine="x86_64")
        checked = seed.execute(plan, check=True)
        self.assertTrue(checked["linux_hint_required"])
        self.assertFalse((self.profile / "WidevineCdm").exists())
        result = seed.execute(plan)
        hint = json.loads((plan.destination.parent / seed.HINT_NAME).read_text())
        self.assertEqual(hint, {"Path": str(plan.destination)})
        self.assertTrue(result["linux_hint_prepared"])
        self.assertEqual(seed.execute(plan)["status"], "already_seeded")

    def test_cli_errors_are_safe_json_without_exception_paths(self):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            status = seed.main(["--source", str(self.root / "secret-source-missing"), "--profile", str(self.profile), "--check"])
        self.assertEqual(status, 2)
        report = json.loads(output.getvalue())
        self.assertEqual(report["error"], "source_missing")
        self.assertNotIn("secret-source-missing", output.getvalue())


if __name__ == "__main__":
    unittest.main()
