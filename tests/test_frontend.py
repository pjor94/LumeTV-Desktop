from pathlib import Path
import json
import shutil
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import check_frontend as frontend


class FrontendBundleTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="lume-frontend-")
        self.root = Path(self.temp.name)
        for name in frontend.fingerprint()["sources"]:
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, target)
        shutil.copytree(ROOT / "web/app/bundle", self.root / "web/app/bundle")
        self.manifest = self.root / "web/app/bundle/manifest.json"

    def tearDown(self):
        self.temp.cleanup()

    def stamp(self):
        self.manifest.write_text(json.dumps(frontend.fingerprint(self.root)))

    def test_source_edit_without_rebuilding_is_rejected(self):
        self.stamp()
        frontend.check(self.root)
        path = self.root / "web/app/src/domain.ts"
        path.write_text(path.read_text() + "\n// changed\n")
        with self.assertRaisesRegex(ValueError, "sources and embedded assets differ"):
            frontend.check(self.root)

    def test_added_chunk_is_rejected_even_if_known_assets_match(self):
        self.stamp()
        (self.root / "web/app/bundle/forgotten.js").write_text("export const value = 1;")
        with self.assertRaisesRegex(ValueError, "explicit Rust asset map"):
            frontend.check(self.root)

    def test_emitted_html_must_use_the_embedded_asset_paths(self):
        path = self.root / "web/app/bundle/index.html"
        path.write_text(path.read_text().replace('/app.js', '/assets/app-hash.js'))
        self.stamp()
        with self.assertRaisesRegex(ValueError, "Missing embedded frontend resource"):
            frontend.check(self.root)
