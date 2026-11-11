# SPDX-License-Identifier: GPL-3.0-or-later
import base64
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
import update_feed


class UpdateFeedTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="lume-update-feed-test-")
        self.root = Path(self.temp.name)
        self.private = self.root / "private/key.pem"
        self.public = self.root / "public.hex"
        update_feed.initialize(self.private, self.public)

    def tearDown(self):
        self.temp.cleanup()

    def test_key_is_private_and_cannot_be_replaced(self):
        self.assertEqual(self.private.stat().st_mode & 0o777, 0o600)
        with self.assertRaises(ValueError):
            update_feed.initialize(self.private, self.public)

    def test_signature_verifies_exact_bytes_and_rejects_a_changed_manifest(self):
        outputs = update_feed.sign_manifest({"version": "1.2.3"}, self.root, self.private.read_text(), self.public)
        signature = self.root / "signature.bin"
        signature.write_bytes(base64.b64decode(outputs[1].read_text()))
        public = self.root / "public.der"
        public.write_bytes(update_feed.SPKI + bytes.fromhex(self.public.read_text().strip()))
        command = ["openssl", "pkeyutl", "-verify", "-rawin", "-pubin", "-keyform", "DER", "-inkey", str(public), "-sigfile", str(signature), "-in", str(outputs[0])]
        self.assertEqual(subprocess.run(command, capture_output=True).returncode, 0)
        outputs[0].write_bytes(outputs[0].read_bytes() + b" ")
        self.assertNotEqual(subprocess.run(command, capture_output=True).returncode, 0)
        self.assertEqual({p.name for p in outputs}, {"update.json", "update.sig"})

    def test_a_different_signing_key_is_refused(self):
        other = self.root / "other.hex"
        other.write_text("00" * 32)
        with self.assertRaises(ValueError):
            update_feed.sign_manifest({}, self.root, self.private.read_text(), other)

    def test_only_matching_complete_packages_enter_the_feed(self):
        for platform in ("macos-arm64", "macos-x64", "windows-x64", "linux-x64"):
            extensions = ["tar.gz", "deb"] if platform == "linux-x64" else ["zip"] if platform == "windows-x64" else ["zip", "dmg"]
            artifacts = []
            for extension in extensions:
                name = f"LumeTV-1.2.3-{platform}.{extension}"
                (self.root / name).write_bytes(b"package fixture")
                artifacts.append({"name": name, "size": 15, "sha256": hashlib.sha256(b"package fixture").hexdigest()})
            (self.root / f"LumeTV-1.2.3-{platform}.build.json").write_text(json.dumps({"version": "1.2.3", "platform": platform, "artifacts": artifacts}))
        manifest = update_feed.build_manifest(self.root, "1.2.3", True, "Note")
        self.assertEqual(len(manifest["targets"]), 4)
        self.assertEqual(manifest["targets"]["linux-x64"]["installer"]["name"], "LumeTV-1.2.3-linux-x64.deb")
        (self.root / "LumeTV-1.2.3-windows-x64.zip").write_bytes(b"corrupt")
        with self.assertRaises(ValueError):
            update_feed.build_manifest(self.root, "1.2.3", True, "Note")


if __name__ == "__main__":
    unittest.main()
