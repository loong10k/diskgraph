"""旧恢复池的源装配检查；不抑制编译警告，也不代替原生行为证明。"""
from pathlib import Path
import sys
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import qualify_windows_legacy_api as support

class LegacySupportTests(unittest.TestCase):
    def test_removes_only_the_unreachable_new_method(self):
        prefix = b"impl Owner {\r\n"
        method = (b"    /// bounded original owner\r\n    #[cfg(windows)]\r\n"
                  b"    pub(crate) fn cleanup_until(&mut self) {\r\n"
                  b"        if live { actual(); }\r\n    }\r\n")
        suffix = b"    fn cleanup() { original(); }\r\n}\r\n"
        self.assertEqual(support.strip_unused_pool_deadline_method(prefix+method+suffix), prefix+suffix)
    def test_no_new_method_keeps_exact_original_bytes(self):
        source = b"original source\r\n"
        self.assertEqual(support.strip_unused_pool_deadline_method(source), source)
    def test_unknown_repeated_or_truncated_method_is_refused(self):
        method = b"    /// bounded\n    pub(crate) fn cleanup_until() {\n    }\n"
        for source in [method*2, b"fn cleanup_until() {}", method[:-6]+b"    fn other() {}\n    }\n"]:
            with self.assertRaises(RuntimeError):
                support.strip_unused_pool_deadline_method(source)
    def test_actual_sources_keep_compatibility_logic(self):
        root = Path(__file__).resolve().parents[2]
        for name in support.POOL_DEADLINE_SUPPORT:
            source = (root/name).read_bytes()
            actual = support.strip_unused_pool_deadline_method(source)
            self.assertNotIn(b"fn cleanup_until(", actual)
            self.assertIn(b"fn cleanup(", actual)
            self.assertLess(len(actual),len(source))

if __name__ == "__main__":
    unittest.main()
