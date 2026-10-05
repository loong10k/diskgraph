import hashlib
import importlib.util
import io
import json
import tarfile
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "qualify_macos_installed_worker.py"
spec = importlib.util.spec_from_file_location("qualifier", SCRIPT)
qualifier = importlib.util.module_from_spec(spec)
spec.loader.exec_module(qualifier)


class MacosInstalledQualifierTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.checkout = Path(self.temp.name)
        self.directory = self.checkout / qualifier.CANDIDATE
        self.directory.mkdir(parents=True)

    def candidate(self, records, declared=None):
        archive = self.directory / "candidate.tar.gz"
        with tarfile.open(archive, "w:gz") as out:
            for name, content, kind in records:
                info = tarfile.TarInfo(name)
                if kind == "file":
                    info.size = len(content)
                    out.addfile(info, io.BytesIO(content))
                else:
                    info.type = tarfile.SYMTYPE
                    info.linkname = "/tmp/escaped"
                    out.addfile(info)
        sources = declared or {name: hashlib.sha256(content).hexdigest() for name, content, _ in records}
        manifest = {"schema_version": 1, "archive_sha256": qualifier.digest(archive), "sources": sources}
        (self.directory / "manifest.json").write_text(json.dumps(manifest))

    def test_regular_frozen_sources_mount_and_match(self):
        self.candidate([("crates/diskgraph-engine/src/frozen.rs", b"source", "file")])
        qualifier.mount(self.checkout)
        self.assertEqual((self.checkout / "crates/diskgraph-engine/src/frozen.rs").read_bytes(), b"source")

    def test_source_escape_and_non_source_inputs_are_rejected(self):
        for name in ["/tmp/escaped", "crates/diskgraph-engine/src/../../escaped", ".git/config", "crates/diskgraph-engine/src/.git/config"]:
            self.candidate([(name, b"bad", "file")])
            with self.assertRaises(ValueError):
                qualifier.mount(self.checkout)

    def test_symbolic_archive_member_is_rejected(self):
        self.candidate([("crates/diskgraph-engine/src/frozen.rs", b"", "symlink")])
        with self.assertRaises(ValueError):
            qualifier.mount(self.checkout)

    def test_digest_failure_never_partially_replaces_existing_source(self):
        a = "crates/diskgraph-engine/src/a.rs"
        b = "crates/diskgraph-engine/src/b.rs"
        existing = self.checkout / a
        existing.parent.mkdir(parents=True, exist_ok=True)
        existing.write_bytes(b"original")
        self.candidate([(a, b"changed", "file"), (b, b"bad", "file")], {a: hashlib.sha256(b"changed").hexdigest(), b: "0" * 64})
        with self.assertRaises(ValueError):
            qualifier.mount(self.checkout)
        self.assertEqual(existing.read_bytes(), b"original")
        self.assertFalse((self.checkout / b).exists())

    def test_duplicate_members_and_changed_archive_are_rejected(self):
        name = "crates/diskgraph-engine/src/a.rs"
        self.candidate([(name, b"a", "file"), (name, b"a", "file")])
        with self.assertRaises(ValueError):
            qualifier.mount(self.checkout)
        self.candidate([(name, b"a", "file")])
        with (self.directory / "candidate.tar.gz").open("ab") as out:
            out.write(b"tampered")
        with self.assertRaises(ValueError):
            qualifier.mount(self.checkout)

    def test_destination_parent_symlink_is_rejected(self):
        outside = self.checkout / "outside"
        outside.mkdir()
        parent = self.checkout / "crates/diskgraph-engine"
        parent.mkdir(parents=True, exist_ok=True)
        (parent / "src").symlink_to(outside, target_is_directory=True)
        self.candidate([("crates/diskgraph-engine/src/a.rs", b"a", "file")])
        with self.assertRaises(ValueError):
            qualifier.mount(self.checkout)
        self.assertFalse((outside / "a.rs").exists())


if __name__ == "__main__":
    unittest.main()
