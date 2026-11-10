import json
from pathlib import Path
import struct
import sys
import tarfile
import tempfile
import unittest
import zipfile

SCRIPTS = Path(__file__).resolve().parents[1] / "scripts"
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))
import prepare_cef_sdk as sdk


class NativeSdkTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="lume-native-sdk-")
        self.root = Path(self.temp.name)

    def tearDown(self):
        self.temp.cleanup()

    def fixture(self, platform):
        pin = dict(sdk.pins()[platform])
        tree = self.root / (platform + "-source")
        (tree / "include").mkdir(parents=True)
        (tree / "Release").mkdir()
        api = ""
        for number, key in [("15000", "stable_hash"), ("999999", "experimental_hash")]:
            api += (f'#define CEF_API_VERSION_{number} {number}\n#if defined(OS_{pin["api_platform"]})\n'
                    f'#define CEF_API_HASH_{number} "{pin[key]}"\n#endif\n')
        api += "#define CEF_API_VERSION_LAST CEF_API_VERSION_15000\n"
        (tree / "include/cef_api_versions.h").write_text(api)
        (tree / "include/cef_version.h").write_text(f'#define CEF_VERSION "{pin["cef_version"]}"\n')
        for name in ["CMakeLists.txt", "LICENSE.txt", "CREDITS.html"]:
            (tree / name).write_text("synthetic fixture")
        binary = sdk.runtime_library(tree, pin)
        binary.parent.mkdir(parents=True, exist_ok=True)
        if pin["os"] == "macos":
            machine = 0x0100000C if pin["arch"] == "aarch64" else 0x01000007
            binary.write_bytes(b"\xcf\xfa\xed\xfe" + struct.pack("<I", machine) + b"synthetic")
        else:
            (tree / "Resources/locales").mkdir(parents=True)
            (tree / "Resources/locales/it.pak").write_bytes(b"locale")
            if pin["os"] == "windows":
                data = bytearray(128)
                data[:2] = b"MZ"
                struct.pack_into("<I", data, 60, 64)
                data[64:68] = b"PE\0\0"
                struct.pack_into("<H", data, 68, 0x8664)
                binary.write_bytes(data)
                (tree / "Release/bootstrap.exe").write_bytes(data)
            else:
                data = bytearray(64)
                data[:6] = b"\x7fELF\x02\x01"
                struct.pack_into("<H", data, 18, 62)
                binary.write_bytes(data)
                (tree / "Release/chrome-sandbox").write_bytes(data)
        archive = self.root / pin["archive"]
        if archive.suffix == ".zip":
            with zipfile.ZipFile(archive, "w") as output:
                for path in tree.rglob("*"):
                    if path.is_file():
                        output.write(path, path.relative_to(tree).as_posix())
        else:
            with tarfile.open(archive, "w:xz") as output:
                for path in tree.rglob("*"):
                    if path.is_file():
                        output.add(path, arcname=path.relative_to(tree), recursive=False)
        pin["sha256"] = sdk.sha256(archive)
        return pin, archive

    def test_all_four_platforms_preserve_versions_and_validate_prepared_cache(self):
        for platform in sdk.pins():
            with self.subTest(platform=platform):
                pin, archive = self.fixture(platform)
                target = sdk.prepare(platform, cache=self.root / "cache", archive=archive, pin=pin)
                metadata = json.loads((target / "archive.json").read_text())
                self.assertIn(pin["cef_version"], metadata["name"])
                self.assertEqual(metadata["lume_obs_provenance"]["sha256"], pin["sha256"])
                self.assertEqual(sdk.prepare(platform, cache=self.root / "cache", archive=archive, pin=pin), target)
                if pin["os"] == "windows":
                    self.assertTrue((target / "bootstrap.exe").is_file())
                    self.assertTrue((target / "libcef.dll").is_file())
                elif pin["os"] == "macos":
                    self.assertTrue((target / "Chromium Embedded Framework.framework").is_symlink())
                else:
                    self.assertTrue((target / "libcef.so").is_file())

    def test_wrong_checksum_prevents_any_sdk_extraction(self):
        pin, archive = self.fixture("linux-x64")
        pin["sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "checksum"):
            sdk.prepare("linux-x64", cache=self.root / "cache", archive=archive, pin=pin)
        self.assertFalse((self.root / "cache/linux-x64/sdk").exists())

    def test_hashes_version_and_architecture_are_independent_requirements(self):
        pin, archive = self.fixture("macos-x64")
        for key, wrong in [("stable_hash", "0" * 40), ("experimental_hash", "0" * 40),
                           ("cef_version", "wrong"), ("arch", "aarch64")]:
            with self.subTest(key=key), self.assertRaises(ValueError):
                sdk.prepare("macos-x64", cache=self.root / ("cache-" + key), archive=archive,
                            pin=dict(pin, **{key: wrong}))

    def test_modified_cached_runtime_is_rejected(self):
        pin, archive = self.fixture("windows-x64")
        target = sdk.prepare("windows-x64", cache=self.root / "cache", archive=archive, pin=pin)
        (target / "Release/libcef.dll").write_bytes(b"modified")
        with self.assertRaisesRegex(ValueError, "modified"):
            sdk.prepare("windows-x64", cache=self.root / "cache", archive=archive, pin=pin)

    def test_arbitrary_existing_sdk_is_not_overwritten(self):
        pin, archive = self.fixture("linux-x64")
        target = self.root / "cache/linux-x64/sdk"
        target.mkdir(parents=True)
        (target / "keep").write_text("retain")
        with self.assertRaisesRegex(ValueError, "not generated"):
            sdk.prepare("linux-x64", cache=self.root / "cache", archive=archive, pin=pin)
        self.assertEqual((target / "keep").read_text(), "retain")

    def test_zip_traversal_is_rejected(self):
        archive = self.root / "unsafe.zip"
        with zipfile.ZipFile(archive, "w") as output:
            output.writestr("../outside", "fixture")
        with self.assertRaisesRegex(ValueError, "Unsafe"):
            sdk.extract_archive(archive, self.root / "extract")
        self.assertFalse((self.root / "outside").exists())

    def test_tar_symlink_cannot_escape_extraction(self):
        archive = self.root / "unsafe.tar.xz"
        with tarfile.open(archive, "w:xz") as output:
            link = tarfile.TarInfo("escape")
            link.type = tarfile.SYMTYPE
            link.linkname = "../outside"
            output.addfile(link)
        destination = self.root / "extract"
        destination.mkdir()
        with self.assertRaises((ValueError, tarfile.TarError)):
            sdk.extract_archive(archive, destination)


if __name__ == "__main__":
    unittest.main()
