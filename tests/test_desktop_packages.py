from pathlib import Path, PureWindowsPath
import os
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile

SCRIPTS = Path(__file__).resolve().parents[1] / "scripts"
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))
import build_desktop as build


class DesktopPackageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="lume-package-test-")
        self.root = Path(self.temp.name)
        (self.root / "Cargo.toml").write_text('[package]\nversion="1.2.3"\n')
        self.sdk = self.root / "sdk"
        self.sdk.mkdir()
        (self.sdk / "locales").mkdir()
        for name in ("icudtl.dat", "resources.pak", "chrome_100_percent.pak", "locales/it.pak",
                     "libcef.dll", "chrome_elf.dll", "bootstrap.exe", "libcef.so", "chrome-sandbox"):
            (self.sdk / name).write_bytes(("fixture-" + name).encode())
        # Never copy an SDK's optional CDM into a distributable.
        (self.sdk / "libwidevinecdm.dll").write_bytes(b"private CDM fixture")
        self.binaries = self.root / "binaries"
        self.binaries.mkdir()
        for name in ("lume_cef_windows.dll", "lume-player.exe", "lume-cef", "lume-player", "lume-updater", "lume-updater.exe"):
            (self.binaries / name).write_bytes(("compiled-" + name).encode())
        self.notices = self.root / "notices"
        self.notices.mkdir()
        (self.notices / "LICENSE").write_text("fixture license")
        icon = self.root / "web/app/assets/app-icon.png"
        icon.parent.mkdir(parents=True)
        icon.write_bytes(b"icon fixture")
        self.stage = self.root / "stage"
        self.stage.mkdir()
        self.output = self.root / "dist"
        self.output.mkdir()

    def tearDown(self):
        self.temp.cleanup()

    def assemble(self, platform):
        return build.assemble(platform, self.sdk, self.binaries, self.stage, self.notices, root=self.root)

    def test_windows_uses_matching_bootstrap_and_dll_stems_with_runtime_and_notices(self):
        payload, executable = self.assemble("windows-x64")
        self.assertEqual(executable.name, "LumeTV.exe")
        self.assertEqual(executable.read_bytes(), (self.sdk / "bootstrap.exe").read_bytes())
        self.assertEqual((payload / "LumeTV.dll").read_bytes(), (self.binaries / "lume_cef_windows.dll").read_bytes())
        self.assertTrue((payload / "locales/it.pak").is_file())
        self.assertTrue((payload / "licenses/LICENSE").is_file())
        self.assertTrue((payload / "lume-updater.exe").is_file())
        self.assertFalse(any("widevine" in p.name.casefold() for p in payload.rglob("*")))
        artifacts = build.archive_package("windows-x64", payload, self.output, root=self.root)
        with zipfile.ZipFile(artifacts[0]) as archive:
            self.assertIn(payload.name + "/LumeTV.exe", archive.namelist())
            self.assertIn(payload.name + "/libcef.dll", archive.namelist())

    def test_linux_launches_from_its_own_directory_with_executable_permissions(self):
        payload, executable = self.assemble("linux-x64")
        self.assertEqual(executable.name, "AppRun")
        self.assertEqual((payload / "lume-updater").stat().st_mode & 0o777, 0o755)
        self.assertEqual(executable.stat().st_mode & 0o777, 0o755)
        self.assertIn('"$LUME_APP_DIR/lume-cef" "$@"', executable.read_text())
        self.assertIn("LD_LIBRARY_PATH", executable.read_text())
        self.assertEqual((payload / "chrome-sandbox").stat().st_mode & 0o7777, 0o755)
        self.assertNotIn("--no-sandbox", executable.read_text())

    def test_windows_zip_accepts_reproducible_sdk_timestamps_without_changing_contents(self):
        source = self.sdk / "libcef.dll"
        os.utime(source, (0, 0))
        payload, _ = self.assemble("windows-x64")
        artifacts = build.archive_package("windows-x64", payload, self.output, root=self.root)
        member = payload.name + "/libcef.dll"
        with zipfile.ZipFile(artifacts[0]) as archive:
            self.assertEqual(archive.getinfo(member).date_time, (1980, 1, 1, 0, 0, 0))
            self.assertEqual(archive.read(member), source.read_bytes())

    def test_linux_deb_records_standard_sandbox_permissions_and_runtime_dependencies(self):
        payload, _ = self.assemble("linux-x64")
        observed = []
        requested_modes = []
        original_chmod = Path.chmod

        def fixture_chmod(path, mode, **kwargs):
            requested_modes.append((path, mode))
            # Fixtures never install a setuid executable on the developer host.
            original_chmod(path, mode & 0o777, **kwargs)

        def fake_dpkg(command, **_kwargs):
            if command[:2] == ["dpkg-deb", "--contents"]:
                return "-rwsr-xr-x root/root 10 2026-01-01 00:00 ./opt/lume-tv/chrome-sandbox\n"
            self.assertEqual(command[:2], ["dpkg-deb", "--root-owner-group"])
            staging = Path(command[-2])
            self.assertIn((staging / "opt/lume-tv/chrome-sandbox", 0o4755), requested_modes)
            control = (staging / "DEBIAN/control").read_text()
            self.assertIn("Version: 1.2.3", control)
            self.assertIn("libc6 (>= 2.39)", control)
            self.assertIn("Exec=/opt/lume-tv/AppRun", (staging / "usr/share/applications/it.lumetv.desktop.desktop").read_text())
            Path(command[-1]).write_bytes(b"deb fixture")
            observed.append(True)

        with patch.object(build, "run", side_effect=fake_dpkg), patch.object(Path, "chmod", fixture_chmod):
            artifacts = build.archive_package("linux-x64", payload, self.output, root=self.root)
        self.assertEqual(len(artifacts), 2)
        self.assertTrue(observed)
        with tarfile.open(artifacts[0]) as archive:
            self.assertEqual(archive.getmember(payload.name + "/AppRun").mode, 0o755)

    def test_missing_cef_resources_prevent_creating_a_partial_download(self):
        (self.sdk / "icudtl.dat").unlink()
        with self.assertRaisesRegex(ValueError, "missing CEF"):
            self.assemble("windows-x64")

    def test_platform_guard_rejects_cross_os_packaging(self):
        with patch.object(build.host_platform, "system", return_value="Darwin"), \
             patch.object(build.host_platform, "machine", return_value="arm64"):
            build.native_host(build.cef_sdk.pins()["macos-arm64"])
            with self.assertRaisesRegex(ValueError, "native runner"):
                build.native_host(build.cef_sdk.pins()["windows-x64"])

    def test_windows_cef_object_paths_fit_msvc_limit_in_github_workspace(self):
        workspace = PureWindowsPath(r"D:\a\LumeTV-Desktop\LumeTV-Desktop")
        generated = PureWindowsPath(
            "x86_64-pc-windows-msvc/release/build/cef-dll-sys-8c9bc9e050925984/out/build/"
            "libcef_dll_wrapper/CMakeFiles/libcef_dll_wrapper.dir/ctocpp/test/"
            "api_version_test_ref_ptr_library_child_child_ctocpp.cc.obj")
        object_path = build.cargo_target_directory("windows-x64", workspace) / generated
        self.assertLess(len(str(object_path)), 260)

    def test_metadata_cannot_hide_a_different_binding_or_api_after_sdk_fallback(self):
        pin = build.cef_sdk.pins()["macos-arm64"]
        info = {"compiled_os": "macos", "compiled_arch": "aarch64", "backend": "embedded_cef",
                "sandbox": True, "cef_expected": "150.0.14+baseline", "cef_api_version": 999999}
        build.validate_runtime_info(info, pin)
        for changed in [dict(info, cef_expected="156.0.15+other"), dict(info, cef_api_version=15000),
                        dict(info, compiled_arch="x86_64"), dict(info, sandbox=False)]:
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                build.validate_runtime_info(changed, pin)


if __name__ == "__main__":
    unittest.main()
