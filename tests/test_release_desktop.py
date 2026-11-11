import json
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1] / "scripts"
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))
import build_desktop as build
import prepare_cef_sdk as sdk
import release_desktop as release

REVISION = "a" * 40


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="lume-release-test-")
        self.root = Path(self.temp.name)
        (self.root / "Cargo.toml").write_text('[package]\nversion="1.2.3"\n')
        self.output = self.root / "dist"
        self.output.mkdir()

    def tearDown(self):
        self.temp.cleanup()

    def fixture_source(self, version="1.2.3"):
        (self.root / "Cargo.toml").write_text(f'[package]\nversion="{version}"\n')
        windows = self.root / "crates/lume-cef-windows"
        windows.mkdir(parents=True, exist_ok=True)
        (windows / "Cargo.toml").write_text(f'[package]\nversion="{version}"\n')
        (self.root / "Cargo.lock").write_text("\n".join(
            f'[[package]]\nname="{name}"\nversion="{version}"\n'
            for name in ("lume-player", "lume-cef-windows")))
        ui = self.root / "web/app"
        ui.mkdir(parents=True, exist_ok=True)
        (ui / "package.json").write_text(json.dumps({"version": version}))
        (ui / "package-lock.json").write_text(json.dumps({"version": version, "packages": {"": {"version": version}}}))
        (self.root / "CHANGELOG.md").write_text(f"# Changelog\n\n## {version}\n\n- Cambiamento fixture.\n\n## 0.0.1\n\n- Prima versione.\n")

    def fixture_assets(self):
        suffixes = {"macos-arm64": [".zip", ".dmg"], "macos-x64": [".zip", ".dmg"],
                    "windows-x64": [".zip"], "linux-x64": [".tar.gz", ".deb"]}
        with patch.object(build, "source_revision", return_value=REVISION):
            for platform, extensions in suffixes.items():
                files = []
                for extension in extensions:
                    path = self.output / (f"LumeTV-1.2.3-{platform}" + extension)
                    path.write_bytes(b"fixture archive")
                    files.append(path)
                build.write_manifest(platform, files, root=self.root)
        archive = self.output / "LumeTV-1.2.3-source.tar.gz"
        archive.write_bytes(b"fixture sources")
        data = {"schema_version": 1, "platform": "source", "version": "1.2.3",
                "source_revision": REVISION, "vendored_cargo_packages": 20,
                "artifacts": [{"name": archive.name, "size": archive.stat().st_size,
                               "sha256": sdk.sha256(archive)}]}
        manifest = self.output / "LumeTV-1.2.3-source.build.json"
        manifest.write_text(json.dumps(data))
        manifest.with_suffix(".sha256").write_text(
            f"{sdk.sha256(archive)}  {archive.name}\n{sdk.sha256(manifest)}  {manifest.name}\n")

    def test_release_requires_an_exact_safe_version_tag(self):
        release.validate_tag("v1.2.3", "1.2.3")
        for tag in ["1.2.3", "v1.2.4", "v1.2.3;command", "v1.2.3\n", "refs/heads/main"]:
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                release.validate_tag(tag, "1.2.3")

    def test_plain_versions_publish_as_latest_and_preview_tags_stay_previews(self):
        for tag, preview in [("v1.2.3", False), ("v1.2.4-beta.1", True), ("v2.0.0-rc.1", True)]:
            with self.subTest(tag=tag):
                command = release.publication_command(tag, self.output, [])
                self.assertEqual("--prerelease" in command, preview)
                self.assertEqual("--latest=false" in command, preview)
                self.assertEqual("--latest" in command, not preview)
                self.assertIn("--verify-tag", command)
                self.assertEqual(command[command.index("--repo") + 1], "pjor94/LumeTV-Desktop")

    def test_release_identity_checks_rust_windows_ui_and_lockfiles(self):
        self.fixture_source()
        with patch.object(build, "source_revision", return_value=REVISION):
            self.assertFalse(release.source_metadata("v1.2.3", root=self.root)["prerelease"])
            for path in ["crates/lume-cef-windows/Cargo.toml", "web/app/package.json", "web/app/package-lock.json", "Cargo.lock"]:
                with self.subTest(path=path):
                    file = self.root / path
                    original = file.read_text()
                    file.write_text(original.replace("1.2.3", "1.2.4"))
                    with self.assertRaises(ValueError):
                        release.source_metadata("v1.2.3", root=self.root)
                    file.write_text(original)
        self.fixture_source("1.2.4-rc.1")
        with patch.object(build, "source_revision", return_value=REVISION):
            self.assertTrue(release.source_metadata("v1.2.4-rc.1", root=self.root)["prerelease"])

    def test_changelog_requires_the_exact_release_heading(self):
        self.fixture_source()
        self.assertEqual(release.changelog_notes("1.2.3", root=self.root), "- Cambiamento fixture.")
        for heading in ["## 1.2.30", "## In sviluppo", "## 1.2.3beta"]:
            (self.root / "CHANGELOG.md").write_text(f"{heading}\n\nVersione 1.2.3 in preparazione.\n")
            with self.assertRaisesRegex(ValueError, "changelog"):
                release.changelog_notes("1.2.3", root=self.root)

    def test_notes_include_actual_changes_and_matching_platform_downloads(self):
        self.fixture_source()
        notes = release.render_notes({"tag": "v1.2.3", "version": "1.2.3", "revision": REVISION, "prerelease": False}, root=self.root)
        self.assertIn("Cambiamento fixture.", notes)
        self.assertIn("LumeTV-1.2.3-macos-arm64.dmg", notes)
        self.assertIn("LumeTV-1.2.3-windows-x64.zip", notes)
        self.assertIn("LumeTV-1.2.3-linux-x64.deb", notes)
        self.assertIn("La 0.1.0 non ha l’updater", notes)
        self.assertNotIn("Versione di anteprima.", notes)

    def test_feed_signature_channel_matches_the_github_release_channel(self):
        archive = self.output / "package.zip"
        archive.write_bytes(b"fixture archive")
        def signature_outputs(*_args):
            paths = [self.output / "update.json", self.output / "update.sig"]
            for path in paths:
                path.write_text("signed fixture")
            return paths
        for version, preview in [("1.2.3", False), ("1.2.4-beta.1", True)]:
            self.fixture_source(version)
            with patch.dict(release.os.environ, {"GITHUB_ACTIONS": "true", "GH_TOKEN": "fixture", "LUME_UPDATE_SIGNING_KEY": "fixture"}), \
                 patch.object(release, "collect", return_value=[archive]), \
                 patch.object(release.update_feed, "build_manifest", return_value={}) as feed, \
                 patch.object(release.update_feed, "sign_manifest", side_effect=signature_outputs), \
                 patch.object(release.subprocess, "run") as run:
                release.publish("v" + version, self.output, root=self.root)
                self.assertEqual(feed.call_args.args[2], preview)
                self.assertEqual("--prerelease" in run.call_args.args[0], preview)
                self.assertIn("update.sig", (self.output / "SHA256SUMS.txt").read_text())

    def test_local_preparation_refuses_uncommitted_source_without_creating_a_tag(self):
        self.fixture_source()
        destination = self.root / ".cache/release"
        with patch.object(build, "source_revision", return_value=REVISION), \
             patch.object(release.subprocess, "check_output", return_value=" M source.rs\n") as git:
            with self.assertRaisesRegex(ValueError, "committed"):
                release.prepare("v1.2.3", destination, root=self.root)
        self.assertEqual(git.call_args.args[0], ["git", "status", "--porcelain"])
        self.assertFalse(destination.exists())

    def test_all_platforms_and_corresponding_sources_are_collected(self):
        self.fixture_assets()
        files = release.verify_assets(self.output, "1.2.3", REVISION)
        self.assertEqual(len(files), 18)
        self.assertIn("LumeTV-1.2.3-source.tar.gz", [p.name for p in files])

    def test_missing_platform_cannot_be_published_as_a_complete_release(self):
        self.fixture_assets()
        (self.output / "LumeTV-1.2.3-windows-x64.build.json").unlink()
        with self.assertRaisesRegex(ValueError, "four native"):
            release.verify_assets(self.output, "1.2.3", REVISION)

    def test_changed_download_is_rejected_even_with_the_same_size(self):
        self.fixture_assets()
        artifact = self.output / "LumeTV-1.2.3-windows-x64.zip"
        artifact.write_bytes(b"x" * artifact.stat().st_size)
        with self.assertRaisesRegex(ValueError, "checksum differs"):
            release.verify_assets(self.output, "1.2.3", REVISION)

    def test_other_commit_or_version_artifacts_are_rejected(self):
        self.fixture_assets()
        for version, revision in [("1.2.4", REVISION), ("1.2.3", "b" * 40)]:
            with self.subTest(version=version), self.assertRaisesRegex(ValueError, "version/commit"):
                release.verify_assets(self.output, version, revision)

    def test_sidecars_and_unexpected_files_are_checked_before_upload(self):
        self.fixture_assets()
        sidecar = self.output / "LumeTV-1.2.3-windows-x64.build.sha256"
        original = sidecar.read_text()
        sidecar.write_text("wrong")
        with self.assertRaisesRegex(ValueError, "sidecar"):
            release.verify_assets(self.output, "1.2.3", REVISION)
        sidecar.write_text(original)
        (self.output / "run.log").write_text("private diagnostic fixture")
        with self.assertRaisesRegex(ValueError, "Unexpected file"):
            release.verify_assets(self.output, "1.2.3", REVISION)

    def test_source_archive_has_portable_metadata_and_reproducible_bytes(self):
        snapshot = self.root / "LumeTV-source"
        snapshot.mkdir()
        (snapshot / "source.rs").write_text("fixture source")
        first, second = self.root / "first.tar.gz", self.root / "second.tar.gz"
        release.archive_source(snapshot, first)
        release.archive_source(snapshot, second)
        self.assertEqual(first.read_bytes(), second.read_bytes())
        with tarfile.open(first) as archive:
            member = archive.getmember("LumeTV-source/source.rs")
            self.assertEqual((member.uid, member.gid, member.mtime), (0, 0, 0))
            self.assertFalse(member.name.startswith("/"))

    def test_publication_is_not_available_from_a_local_unauthenticated_run(self):
        with patch.dict(release.os.environ, {"GITHUB_ACTIONS": "false", "GH_TOKEN": ""}):
            with self.assertRaisesRegex(ValueError, "reserved"):
                release.publish("v1.2.3", self.output)


if __name__ == "__main__":
    unittest.main()
