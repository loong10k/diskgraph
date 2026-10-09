"""ELF需求必须真实绑定声明的glibc基线，异常元数据不能产生兼容证明。"""
import importlib.util
from pathlib import Path
import struct
import sys
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
SPEC = importlib.util.spec_from_file_location('linux_abi', ROOT / 'scripts/linux_abi_requirements.py')


def fixture(versions, machine=183):
    """生成有真实ELF64节布局的最小版本需求夹具，不声称它是可执行程序。"""
    names = b'\0.shstrtab\0.dynstr\0.gnu.version_r\0'
    strings = bytearray(b'\0libc.so.6\0')
    offsets = []
    for version in versions:
        offsets.append(len(strings))
        strings.extend(version.encode('ascii') + b'\0')
    needs = bytearray(struct.pack('<HHIII', 1, len(versions), 1, 16, 0))
    for i, offset in enumerate(offsets):
        needs.extend(struct.pack('<IHHII', 0, 0, i + 2, offset,
                                 16 if i + 1 < len(offsets) else 0))
    header = bytearray(64)
    header[:16] = b'\x7fELF\x02\x01\x01' + b'\0' * 9
    struct.pack_into('<HHI', header, 16, 3, machine, 1)
    struct.pack_into('<Q', header, 40, 64)
    struct.pack_into('<HHH', header, 58, 64, 4, 1)
    payload = bytes(names) + bytes(strings) + bytes(needs)
    base = 64 + 4 * 64
    sections = [bytes(64)]
    for name, kind, offset, size, link in [
        (1, 3, base, len(names), 0),
        (11, 3, base + len(names), len(strings), 0),
        (19, 0x6ffffffe, base + len(names) + len(strings), len(needs), 2),
    ]:
        sections.append(struct.pack('<IIQQQQIIQQ', name, kind, 0, 0, offset, size, link, 0, 1, 0))
    return bytes(header) + b''.join(sections) + payload


class GnuAbiTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.module = importlib.util.module_from_spec(SPEC)
        SPEC.loader.exec_module(cls.module)

    def test_numeric_versions_are_ordered_numerically(self):
        self.assertEqual(self.module.glibc_requirements(fixture(['GLIBC_2.9', 'GLIBC_2.17'])),
                         ['2.9', '2.17'])

    def test_package_above_declared_baseline_is_refused(self):
        with self.assertRaisesRegex(ValueError, '2.39'):
            self.module.require_baseline(fixture(['GLIBC_2.17', 'GLIBC_2.39']), '2.17')

    def test_exact_baseline_is_allowed(self):
        self.assertEqual(self.module.require_baseline(fixture(['GLIBC_2.17']), '2.17'), ['2.17'])

    def test_x86_and_arm_native64_have_same_version_semantics(self):
        for machine in (62, 183):
            self.assertEqual(self.module.glibc_requirements(fixture(['GLIBC_2.17'], machine)), ['2.17'])

    def test_truncation_at_header_section_or_string_is_refused(self):
        data = fixture(['GLIBC_2.17'])
        for length in (0, 63, 100, len(data) - 1):
            with self.subTest(length=length), self.assertRaises(ValueError):
                self.module.glibc_requirements(data[:length])

    def test_unknown_glibc_requirement_is_refused(self):
        for version in ('GLIBC_PRIVATE', 'GLIBC_ABI_DT_RELR', 'GLIBC_2.x'):
            with self.subTest(version=version), self.assertRaises(ValueError):
                self.module.glibc_requirements(fixture([version]))

    def test_unsupported_machine_and_endianness_are_refused(self):
        data = bytearray(fixture(['GLIBC_2.17']))
        data[5] = 2
        for raw in (fixture(['GLIBC_2.17'], 40), bytes(data)):
            with self.assertRaises(ValueError):
                self.module.glibc_requirements(raw)

    def test_invalid_baseline_and_no_glibc_proof_are_refused(self):
        for baseline in ('2.x', '2.17\n', '-2.17'):
            with self.assertRaises(ValueError):
                self.module.require_baseline(fixture(['GLIBC_2.17']), baseline)
        with self.assertRaises(ValueError):
            self.module.glibc_requirements(fixture(['OTHER_1.0']))

    def test_relocatable_or_unknown_object_cannot_claim_a_package_abi(self):
        for kind in (0, 1, 4):
            raw = bytearray(fixture(['GLIBC_2.17']))
            struct.pack_into('<H', raw, 16, kind)
            with self.subTest(kind=kind), self.assertRaises(ValueError):
                self.module.glibc_requirements(raw)


if __name__ == '__main__':
    unittest.main()
