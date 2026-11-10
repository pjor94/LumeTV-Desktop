# SPDX-License-Identifier: GPL-3.0-or-later
"""Synthetic SDK/archive tests. No browser binary is downloaded or executed."""

import importlib.util
import json
from pathlib import Path
import struct
import sys
import tarfile
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "prepare_obs_cef.py"
SPEC = importlib.util.spec_from_file_location("lume_prepare_obs_cef", SCRIPT)
prepare = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = prepare
SPEC.loader.exec_module(prepare)


class PrepareObsCefTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name).resolve()
        self.cache = self.root / ".cache" / "cef-obs"
        self.sdk = self.cache / "sdk"
        self.sdk.mkdir(parents=True)
        (self.sdk / "include").mkdir()
        (self.sdk / "cmake").mkdir()
        (self.sdk / "libcef_dll").mkdir()
        api = (
            "#define CEF_API_VERSION_15000 15000\n#if defined(OS_WIN)\n#define CEF_API_HASH_15000 \"windows\"\n#elif defined(OS_MAC)\n"
            f"#define CEF_API_HASH_15000 \"{prepare.MAC_STABLE_HASH}\"\n#endif\n"
            "#define CEF_API_VERSION_999999 999999\n#if defined(OS_WIN)\n#define CEF_API_HASH_999999 \"windows\"\n#elif defined(OS_MAC)\n"
            f"#define CEF_API_HASH_999999 \"{prepare.MAC_EXPERIMENTAL_HASH}\"\n#endif\n"
            "#define CEF_API_VERSION_LAST CEF_API_VERSION_15000\n"
        )
        (self.sdk / "include/cef_api_versions.h").write_text(api)
        (self.sdk / "include/cef_version.h").write_text(f'#define CEF_VERSION "{prepare.ACTUAL_CEF_VERSION}"\n')
        (self.sdk / "CMakeLists.txt").write_text("# synthetic CMake fixture\n")
        (self.sdk / "LICENSE.txt").write_text("synthetic license\n")
        (self.sdk / "CREDITS.html").write_text("synthetic credits\n")
        framework = self.sdk / "Release" / prepare.FRAMEWORK
        framework.mkdir(parents=True)
        self.binary = framework / "Chromium Embedded Framework"
        self.binary.write_bytes(b"\xcf\xfa\xed\xfe" + struct.pack("<I", 0x0100000C) + b"synthetic-not-executable")
        self.archive = self.cache / "cef_binary_7871_macos_arm64.tar.xz"
        with tarfile.open(self.archive, "w:xz") as output:
            for path in self.sdk.rglob("*"):
                if path.is_file():
                    output.add(path, arcname=path.relative_to(self.sdk), recursive=False)
        self.expected_sha256 = prepare.hash_file(self.archive)[0]

    def tearDown(self):
        self.temporary.cleanup()

    def run_prepare(self, *, check=False):
        return prepare.prepare(self.archive, self.sdk, check=check, cache_root=self.cache, expected_sha256=self.expected_sha256)

    def test_check_does_not_write_and_normalization_keeps_actual_version_and_no_binary_copy(self):
        before = self.binary.read_bytes()
        checked = self.run_prepare(check=True)
        self.assertEqual(checked["status"], "checked")
        self.assertFalse((self.sdk / "archive.json").exists())
        self.assertFalse((self.sdk / prepare.FRAMEWORK).is_symlink())
        result = self.run_prepare()
        self.assertEqual(result["status"], "prepared")
        metadata = json.loads((self.sdk / "archive.json").read_text())
        self.assertTrue(metadata["name"].startswith("cef_binary_150.0.19+gcf60f42+"))
        self.assertFalse(metadata["name"].startswith("cef_binary_150.0.14+"))
        self.assertEqual(metadata["lume_obs_provenance"]["source_sha256"], self.expected_sha256)
        self.assertEqual(metadata["lume_obs_provenance"]["experimental_mac_api_hash"], prepare.MAC_EXPERIMENTAL_HASH)
        alias = self.sdk / prepare.FRAMEWORK
        self.assertTrue(alias.is_symlink())
        self.assertEqual(alias.readlink(), Path("Release") / prepare.FRAMEWORK)
        self.assertEqual(self.binary.read_bytes(), before)
        self.assertFalse(result["framework_duplicated"])
        self.assertFalse(result["drm_verified"])
        self.assertEqual(self.run_prepare()["status"], "prepared")

    def test_archive_checksum_is_required_before_any_normalization(self):
        with self.assertRaisesRegex(prepare.PrepareError, "obs_archive_sha256_mismatch"):
            prepare.prepare(self.archive, self.sdk, cache_root=self.cache, expected_sha256="0" * 64)
        self.assertFalse((self.sdk / "archive.json").exists())

    def test_stable_experimental_and_actual_versions_are_independently_checked(self):
        api = (self.sdk / "include/cef_api_versions.h").read_text()
        version = (self.sdk / "include/cef_version.h").read_text()
        for wrong_api, wrong_version in [
            (api.replace(prepare.MAC_STABLE_HASH, "0" * 40), version),
            (api.replace(prepare.MAC_EXPERIMENTAL_HASH, "0" * 40), version),
            (api.replace("LAST CEF_API_VERSION_15000", "LAST CEF_API_VERSION_15600"), version),
            (api, version.replace("150.0.19", "150.0.14")),
        ]:
            with self.assertRaises(prepare.PrepareError):
                prepare.validate_api_headers(wrong_api, wrong_version)

    def test_sdk_binary_must_match_verified_archive_even_when_api_headers_match(self):
        self.binary.write_bytes(self.binary.read_bytes() + b"unexpected-changes")
        with self.assertRaisesRegex(prepare.PrepareError, "sdk_archive_content_mismatch"):
            self.run_prepare()
        self.assertFalse((self.sdk / "archive.json").exists())

    def test_incompatible_framework_alias_is_not_replaced(self):
        alias = self.sdk / prepare.FRAMEWORK
        alias.symlink_to("unrelated-framework", target_is_directory=True)
        with self.assertRaisesRegex(prepare.PrepareError, "existing_framework_alias_conflict"):
            self.run_prepare()
        self.assertEqual(alias.readlink(), Path("unrelated-framework"))


if __name__ == "__main__":
    unittest.main()
