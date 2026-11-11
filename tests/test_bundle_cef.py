import importlib.util
import json
from pathlib import Path
import plistlib
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "scripts/bundle_cef.py"
SPEC = importlib.util.spec_from_file_location("bundle_cef", SCRIPT)
bundler = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(bundler)


class BundleTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="lume-bundle-test-")
        self.root = Path(self.temp.name)
        self.sdk = self.root / "sdk"
        self.target = self.root / "target/debug"
        self.target.mkdir(parents=True)
        for name in ("lume-cef", "lume-cef-helper", "lume-updater"):
            (self.target / name).write_bytes(b"fixture executable")
        (self.root / "LICENSE").write_text("GPL fixture")
        (self.root / "THIRD_PARTY_NOTICES.md").write_text("Notices fixture")
        (self.sdk / "include").mkdir(parents=True)
        (self.sdk / "include/cef_version.h").write_text(
            '#define CEF_VERSION "156.0.15+gfixture+chromium-156.0.8078.80"\n'
            '#define CHROME_VERSION_MAJOR 156\n#define CHROME_VERSION_MINOR 0\n'
            '#define CHROME_VERSION_BUILD 8078\n#define CHROME_VERSION_PATCH 80\n'
        )
        framework = self.sdk / bundler.FRAMEWORK
        (framework / "Versions/A/Resources").mkdir(parents=True)
        (framework / "Versions/A/Chromium Embedded Framework").write_bytes(b"fixture library")
        (framework / "Versions/Current").symlink_to("A", target_is_directory=True)
        (framework / "Chromium Embedded Framework").symlink_to(
            "Versions/Current/Chromium Embedded Framework"
        )
        (framework / "WidevineCdm").mkdir()
        (framework / "WidevineCdm/manifest.json").write_text("private CDM fixture")
        (framework / "Versions/A/libwidevinecdm.dylib").write_bytes(b"private fixture")
        (self.sdk / "CREDITS.html").write_text("CEF credits fixture")
        self.output = self.target / "lume-cef.app"

    def tearDown(self):
        self.temp.cleanup()

    def build(self, **kwargs):
        return bundler.build_bundle(
            self.sdk, self.target, self.output,
            project_root=self.root, skip_sign=True, **kwargs,
        )

    def test_layout_versions_symlinks_and_credits_without_a_cdm(self):
        output = self.build()
        info = plistlib.loads((output / "Contents/Info.plist").read_bytes())
        self.assertEqual(info["CFBundleExecutable"], "lume-cef")
        self.assertFalse(info["LSUIElement"])
        for suffix in bundler.HELPERS:
            name = f"lume-cef {suffix}"
            helper = output / "Contents/Frameworks" / f"{name}.app"
            helper_info = plistlib.loads((helper / "Contents/Info.plist").read_bytes())
            self.assertTrue(helper_info["LSUIElement"])
            self.assertTrue((helper / "Contents/MacOS" / name).is_file())
        framework = output / "Contents/Frameworks" / bundler.FRAMEWORK
        self.assertTrue((framework / "Versions/Current").is_symlink())
        self.assertTrue((framework / "Chromium Embedded Framework").is_symlink())
        self.assertFalse(any("widevine" in p.name.casefold() for p in output.rglob("*")))
        metadata = json.loads((output / "Contents/Resources/runtime.json").read_text())
        self.assertFalse((output / "Contents/MacOS/runtime.json").exists())
        self.assertEqual(metadata["chromium_version"], "156.0.8078.80")
        self.assertEqual(metadata["codec_build"], "stock")
        self.assertFalse(metadata["widevine_cdm_bundled"])
        self.assertEqual(metadata["signature_kind"], "unsigned_fixture")
        self.assertTrue((output / "Contents/Resources/licenses/LICENSE").is_file())
        self.assertTrue((output / "Contents/Resources/licenses/CEF-CREDITS.html").is_file())

    def test_release_framework_layout_and_custom_codec_label(self):
        source = self.sdk / bundler.FRAMEWORK
        (self.sdk / "Release").mkdir()
        source.rename(self.sdk / "Release" / bundler.FRAMEWORK)
        self.build(codec_build="custom")
        metadata = json.loads((self.output / "Contents/Resources/runtime.json").read_text())
        self.assertEqual(metadata["codec_build"], "custom")
        self.assertEqual(metadata["codec_capabilities"], "not_tested")

    def test_product_bundle_uses_original_lume_icon_and_preserves_diagnostic_bundle(self):
        diagnostic = self.build()
        icon = self.root / "web/app/assets/LumeTV.icns"
        icon.parent.mkdir(parents=True)
        icon.write_bytes(b"icns fixture")
        self.output = self.target / "Lume TV.app"
        output = self.build()
        info = plistlib.loads((output / "Contents/Info.plist").read_bytes())
        self.assertEqual(info["CFBundleDisplayName"], "Lume TV")
        self.assertEqual(info["CFBundleIconFile"], "LumeTV.icns")
        self.assertEqual((output / "Contents/Resources/LumeTV.icns").read_bytes(), b"icns fixture")
        self.assertTrue(diagnostic.is_dir())
        self.assertFalse(any("widevine" in p.name.casefold() for p in output.rglob("*")))

    def test_rebuild_replaces_only_its_own_generated_bundle(self):
        self.build()
        (self.output / "old-file").write_text("old")
        self.build()
        self.assertFalse((self.output / "old-file").exists())
        self.assertFalse(list(self.target.glob(".lume-cef-*.app")))

    def test_arbitrary_app_and_outside_project_outputs_are_refused(self):
        self.output.mkdir()
        sentinel = self.output / "keep.txt"
        sentinel.write_text("retain")
        with self.assertRaises(ValueError):
            self.build()
        self.assertEqual(sentinel.read_text(), "retain")
        with self.assertRaises(ValueError):
            bundler.build_bundle(
                self.sdk, self.target, self.root.parent / "lume-cef.app",
                project_root=self.root, skip_sign=True,
            )

    def test_release_version_notices_and_runtime_metadata_are_portable(self):
        (self.root / "Cargo.toml").write_text('[package]\nversion="2.5.9-beta.1"\n')
        notices = self.root / "notices"
        notices.mkdir()
        (notices / "Dependency-LICENSE").write_text("fixture dependency notice")
        output = self.build(additional_licenses=notices)
        info = plistlib.loads((output / "Contents/Info.plist").read_bytes())
        self.assertEqual(info["CFBundleShortVersionString"], "2.5.9")
        metadata = json.loads((output / "Contents/Resources/runtime.json").read_text())
        self.assertEqual(metadata["app_version"], "2.5.9-beta.1")
        self.assertNotIn("sdk_path", metadata)
        self.assertTrue((output / "Contents/Resources/licenses/dependencies/Dependency-LICENSE").is_file())


if __name__ == "__main__":
    unittest.main()
