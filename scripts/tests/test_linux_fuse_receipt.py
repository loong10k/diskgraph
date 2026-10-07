"""FUSE 验收记录不能以旧案例、缺失正控或未回收记录冒充通过。"""
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location('fuse_qualification', ROOT / 'scripts/qualify_linux_fuse_no_recall.py')
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
FIXTURE = ROOT / 'docs/benchmarks/linux_content_no_recall_audit_2026_10_08'


class ReceiptTests(unittest.TestCase):
    """使用保留的真实产品及提供方日志验证证据门禁。"""

    def setUp(self):
        self.text = (FIXTURE / 'nested_orchestration.log').read_text()
        self.product = (FIXTURE / 'current_product.log').read_text()
        self.provider = (FIXTURE / 'current_provider.log').read_text()

    def test_actual_four_cases_are_accepted(self):
        MODULE.verify_output(self.text, self.product, self.provider)

    def test_old_two_cases_do_not_satisfy_nested_acceptance(self):
        with self.assertRaises(RuntimeError):
            MODULE.verify_output(self.text, (FIXTURE / 'direct_current_product.log').read_text(), self.provider)

    def test_missing_original_retirement_is_refused(self):
        with self.assertRaises(RuntimeError):
            MODULE.verify_output('PRODUCT_TEST_EXIT 0', self.product, self.provider)

    def test_missing_positive_control_is_refused(self):
        with self.assertRaises(RuntimeError):
            MODULE.verify_output(self.text, self.product, 'PROVIDER_MOUNTED')

    def test_product_content_callback_is_refused(self):
        with self.assertRaises(RuntimeError):
            MODULE.verify_output(self.text, self.product.replace('before=0 after=0 safe=true', 'before=0 after=1 safe=true', 1), self.provider)


if __name__ == '__main__':
    unittest.main()
