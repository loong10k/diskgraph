"""自动换行开启的真实 Git checkout 仍保留受审源码与上游原始字节。"""
from pathlib import Path
import subprocess
import tempfile
import unittest


ATTRIBUTES = Path(__file__).resolve().parents[2] / ".gitattributes"


class SourceCheckoutBytesTests(unittest.TestCase):
    def checkout(self, files):
        with tempfile.TemporaryDirectory(prefix="dg-checkout-bytes-") as directory:
            root = Path(directory)
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            subprocess.run(["git", "-C", str(root), "config", "core.autocrlf", "true"], check=True)
            (root / ".gitattributes").write_bytes(ATTRIBUTES.read_bytes())
            for name, contents in files.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(contents)
            subprocess.run(["git", "-C", str(root), "add", "."], check=True, capture_output=True)
            for name in files:
                (root / name).unlink()
            subprocess.run(["git", "-C", str(root), "checkout-index", "--all"], check=True)
            return {name: (root / name).read_bytes() for name in files}

    def test_hashed_sources_keep_lf_with_autocrlf_enabled(self):
        files = {
            "scripts/qualification.py": b"# source identity\nprint('fixture')\n",
            "crates/example/src/lib.rs": b"// source identity\npub struct Example;\n",
            "crates/example/integration_candidates/change.diff": b"--- a/example\n+++ b/example\n",
            "scripts/qualification.json": b'{"source": "fixture"}\n',
        }
        self.assertEqual(self.checkout(files), files)

    def test_pinned_vendor_and_binary_bytes_are_unchanged(self):
        files = {
            "crates/diskgraph-disktree-core/raw.rs": b"// preserve upstream\r\n",
            "docs/benchmarks/captured.stdout": b"raw observation\r\n",
            "fixtures/binary.dat": bytes(range(256)),
        }
        self.assertEqual(self.checkout(files), files)


if __name__ == "__main__":
    unittest.main(verbosity=2)
