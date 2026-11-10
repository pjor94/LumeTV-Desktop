import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "scripts/check_repository.py"
SPEC = importlib.util.spec_from_file_location("check_repository", SCRIPT)
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


class RepositoryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="lume-source-test-")
        self.root = Path(self.temp.name)

    def tearDown(self):
        self.temp.cleanup()

    def write(self, name, content):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
        return name

    def test_forced_private_files_and_binaries_are_rejected_before_reading(self):
        names = [
            "profiles/session/preferences.json", "reports/raw.json",
            "target/debug/lume-player", ".env.production", "certs/signing.p12",
            "runtime/WidevineCdm/manifest.json", "runtime/libwidevinecdm.dylib",
        ]
        findings = checker.audit_files(self.root, names)
        self.assertEqual(len(findings), len(names))
        self.assertTrue(all("private or generated" in item for item in findings))

    def test_vendor_distribution_and_reports_placeholder_remain_publishable(self):
        names = [self.write("vendor/shaka/dist/player.js", "source"),
                 self.write("reports/.gitkeep", ""), self.write(".env.example", "PORT=0")]
        self.assertEqual(checker.audit_files(self.root, names), [])

    def test_credentials_are_reported_without_echoing_their_values(self):
        token = "ghp_" + "a" * 36
        name = self.write("src/leak.txt", "value=" + token)
        findings = checker.audit_files(self.root, [name])
        self.assertEqual(len(findings), 1)
        self.assertIn("src/leak.txt:1", findings[0])
        self.assertNotIn(token, findings[0])

    def test_link_to_ignored_or_unpublished_existing_file_is_rejected(self):
        self.write("reports/private.json", "{}")
        name = self.write("README.md", "[Report](reports/private.json)")
        self.assertIn("absent from source snapshot", checker.audit_files(self.root, [name])[0])

    def test_relative_links_images_anchors_and_code_examples(self):
        name = self.write("docs/README.md", """[Guide](../README.md#usage)
![Image](image.png)
[Directory](../src/)
```md
[Example](missing-file.md)
```
[Remote](https://example.org/guide)
""")
        self.write("README.md", "# Usage")
        self.write("docs/image.png", "fixture")
        self.write("src/lib.rs", "source")
        names = [name, "README.md", "docs/image.png", "src/lib.rs"]
        self.assertEqual(checker.audit_files(self.root, names), [])

    def test_links_cannot_escape_the_repository(self):
        name = self.write("README.md", "[Outside](../../outside.md)")
        self.assertIn("outside repository", checker.audit_files(self.root, [name])[0])

    def test_evidence_rejects_urls_and_private_fields_at_any_depth(self):
        for content in ['{"run":{"headers":{}}}', '{"runs":[{"token":"fixture"}]}',
                        '{"source":"https://example.org/private"}']:
            with self.subTest(content=content):
                name = self.write("docs/evidence/run.json", content)
                self.assertIn("private metadata", checker.audit_files(self.root, [name])[0])

    def test_large_files_are_rejected(self):
        name = self.write("archive.txt", "x" * (checker.MAX_FILE_BYTES + 1))
        self.assertIn("exceeds source file limit", checker.audit_files(self.root, [name])[0])

    def test_symlinks_are_not_followed(self):
        self.write("local.txt", "private")
        (self.root / "alias.txt").symlink_to("local.txt")
        self.assertIn("source symlink", checker.audit_files(self.root, ["alias.txt"])[0])

    def test_git_inventory_includes_forced_ignored_files(self):
        subprocess.run(["git", "init", "--quiet"], cwd=self.root, check=True)
        self.write(".gitignore", "/profiles/\n")
        self.write("profiles/forced.json", "{}")
        subprocess.run(["git", "add", "-f", "profiles/forced.json"],
                       cwd=self.root, check=True, capture_output=True)
        names = checker.git_files(self.root, tracked_only=True)
        self.assertEqual(names, ["profiles/forced.json"])
        self.assertIn("private or generated", checker.audit_files(self.root, names)[0])

    def test_staged_content_cannot_be_hidden_by_a_later_worktree_edit(self):
        subprocess.run(["git", "init", "--quiet"], cwd=self.root, check=True)
        self.write("source.txt", "staged version")
        subprocess.run(["git", "add", "source.txt"],
                       cwd=self.root, check=True, capture_output=True)
        self.assertTrue(checker.index_matches_worktree(self.root))
        self.write("source.txt", "different working version")
        self.assertFalse(checker.index_matches_worktree(self.root))


if __name__ == "__main__":
    unittest.main()
