"""监督安装身份与真实制品准入回归；纯单元检查不代替 root 原生安装。"""
import importlib.util
from pathlib import Path
import unittest
import gzip
import hashlib
import io
import json
import os
import tarfile
import tempfile

SPEC = importlib.util.spec_from_file_location(
    'prepare_linux_supervisor_installation',
    Path(__file__).resolve().parents[1] / 'prepare_linux_supervisor_installation.py')
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class IdentityTests(unittest.TestCase):
    def test_dedicated_service_cannot_be_an_allowed_frontend(self):
        with self.assertRaises(ValueError):
            MODULE.validate_identities(1000, 1000, [1000, 1001])

    def test_root_and_shared_groups_cannot_be_service_identity(self):
        for uid, gid in [(0, 1000), (1000, 0)]:
            with self.subTest(uid=uid, gid=gid), self.assertRaises(ValueError):
                MODULE.validate_identities(uid, gid, [1001])

    def test_frontend_list_is_bounded_unique_and_nonempty(self):
        for users in [[], [1001, 1001], [0], list(range(1001, 1066))]:
            with self.subTest(users=users), self.assertRaises(ValueError):
                MODULE.validate_identities(1000, 1000, users)

    def test_only_integer_kernel_identities_are_accepted(self):
        for uid in [True, -1, (1 << 32) - 1, 1 << 32, '1000']:
            with self.subTest(uid=uid), self.assertRaises(ValueError):
                MODULE.validate_identities(uid, 1000, [1001])

    def test_valid_frontend_policy_does_not_alias_caller_list(self):
        users = [1001, 1002]
        actual = MODULE.validate_identities(1000, 1000, users)
        users.append(1000)
        self.assertEqual(actual['frontend_uids'], [1001, 1002])


class BrokerArtifactTests(unittest.TestCase):
    def images(self):
        return {name: {'sha256': str(index) * 64} for index, name in enumerate(
            ['diskgraph', 'diskgraph-scan-worker', 'diskgraph-supervisor'], start=1)}

    def test_fixed_config_uses_actual_installed_images_and_nested_service_data(self):
        policy = MODULE.validate_identities(23111, 23111, [23112])
        config, unit = MODULE.broker_artifacts(Path('/var/lib/dg-test'), policy, self.images())
        self.assertEqual(config['frontend_uid'], 23112)
        self.assertEqual(config['data_dir'], '/var/lib/dg-test/state/data')
        self.assertEqual(config['cli_sha256'], self.images()['diskgraph']['sha256'])
        self.assertEqual(config['worker_sha256'], self.images()['diskgraph-scan-worker']['sha256'])
        self.assertIn('/images/diskgraph-supervisor', unit)
        self.assertIn('Restart=no', unit.splitlines())
        self.assertIn('[Service]', unit.splitlines())
        self.assertNotIn('systemctl', unit)

    def test_broker_preserves_only_birth_and_retirement_capabilities(self):
        policy = MODULE.validate_identities(23111, 23111, [23112])
        _, unit = MODULE.broker_artifacts(Path('/var/lib/dg-test'), policy, self.images())
        properties = dict(line.split('=', 1) for line in unit.splitlines() if '=' in line)
        self.assertEqual(set(properties.get('CapabilityBoundingSet', '').split()),
                         {'CAP_SETUID', 'CAP_SETGID', 'CAP_KILL'})
        self.assertEqual(properties.get('AmbientCapabilities'), 'CAP_SETUID')
        self.assertEqual(properties.get('NoNewPrivileges'), 'yes')
        self.assertEqual(properties.get('RestrictAddressFamilies'), 'AF_UNIX')
        self.assertEqual(properties.get('ProtectSystem'), 'strict')

    def test_multi_frontend_and_unsafe_unit_paths_are_refused_before_deployment(self):
        policy = MODULE.validate_identities(23111, 23111, [23112])
        for prefix in ['/var/lib/dg\nInjected=yes', '/var/lib/' + 'x' * 100, 'relative']:
            with self.subTest(prefix=prefix), self.assertRaises(ValueError):
                MODULE.broker_artifacts(Path(prefix), policy, self.images())
        with self.assertRaises(ValueError):
            MODULE.broker_artifacts(Path('/var/lib/dg-test'),
                MODULE.validate_identities(23111, 23111, [23112, 23113]), self.images())

    def test_systemd_literal_paths_preserve_spaces_quotes_and_percent(self):
        policy = MODULE.validate_identities(23111, 23111, [23112])
        _, unit = MODULE.broker_artifacts(Path('/var/lib/dg %n "quoted"'), policy, self.images())
        self.assertIn('%%n', unit)
        self.assertIn('\\"quoted\\"', unit)


class ArchiveTests(unittest.TestCase):
    target = 'aarch64-unknown-linux-gnu'

    def package(self, extra=None, bad_worker=False, invalid_elf=False, wrong_machine=False, with_broker=False):
        raw = io.BytesIO()
        header = bytearray(64)
        header[:7] = b'\x7fELF\x02\x01\x01'
        header[16:18] = (3).to_bytes(2, 'little')
        header[18:20] = (183).to_bytes(2, 'little')
        header[20:24] = (1).to_bytes(4, 'little')
        names = (*MODULE.NAMES, 'diskgraph-supervisor') if with_broker else MODULE.NAMES
        images = {name: bytes(header) + (name + '-fixture').encode() for name in names}
        if invalid_elf:
            images['diskgraph'] = b'not-an-ELF'
        if wrong_machine:
            broken = bytearray(images['diskgraph'])
            broken[18:20] = (62).to_bytes(2, 'little')
            images['diskgraph'] = bytes(broken)
        worker = images['diskgraph-scan-worker']
        manifest = {'schema_version': 1, 'target': self.target, 'protocol_version': 2,
                    'pinned_scanner_revision': MODULE.PIN,
                    'executable': {'name': 'diskgraph-scan-worker', 'bytes': len(worker),
                                   'sha256': ('0' * 64 if bad_worker else hashlib.sha256(worker).hexdigest())}}
        with tarfile.open(fileobj=raw, mode='w') as archive:
            for name, data in [*images.items(), ('scan-worker-manifest.json', json.dumps(manifest).encode())]:
                entry = tarfile.TarInfo('diskgraph-' + self.target + '/bin/' + name)
                entry.size = len(data)
                archive.addfile(entry, io.BytesIO(data))
            if extra:
                archive.addfile(*extra)
        return io.BytesIO(gzip.compress(raw.getvalue()))

    def install(self, archive, with_broker=False):
        with tempfile.TemporaryDirectory() as directory:
            fd = os.open(directory, os.O_RDONLY)
            try:
                result, manifest = MODULE.install_images(archive, fd, self.target, with_broker=with_broker)
                for name, metadata in result.items():
                    self.assertEqual((Path(directory) / name).stat().st_size, metadata['bytes'])
                    self.assertEqual(metadata['mode'], 0o555)
                    self.assertEqual(metadata['links'], 1)
                return result, manifest
            finally:
                os.close(fd)

    def test_exact_fixed_images_and_manifest_are_copied(self):
        result, _ = self.install(self.package())
        self.assertEqual(set(result), set(MODULE.NAMES))

    def test_broker_installation_requires_actual_fourth_image(self):
        with self.assertRaises(ValueError):
            self.install(self.package(), with_broker=True)
        result, _ = self.install(self.package(with_broker=True), with_broker=True)
        self.assertEqual(set(result), set(MODULE.NAMES) | {'diskgraph-supervisor'})

    def test_old_preparation_accepts_optional_broker_without_enabling_it(self):
        result, _ = self.install(self.package(with_broker=True))
        self.assertIn('diskgraph-supervisor', result)

    def test_non_elf_role_image_is_refused(self):
        with self.assertRaises(ValueError):
            self.install(self.package(invalid_elf=True))

    def test_role_machine_must_match_the_native_target(self):
        with self.assertRaises(ValueError):
            self.install(self.package(wrong_machine=True))

    def test_archive_traversal_and_links_are_rejected(self):
        for name, kind in [('diskgraph-' + self.target + '/../escape', tarfile.REGTYPE),
                           ('diskgraph-' + self.target + '/bin/link', tarfile.SYMTYPE)]:
            entry = tarfile.TarInfo(name)
            entry.type = kind
            entry.linkname = '/outside'
            with self.subTest(name=name), self.assertRaises(ValueError):
                self.install(self.package((entry, io.BytesIO())))

    def test_duplicate_image_cannot_replace_an_exclusive_copy(self):
        entry = tarfile.TarInfo('diskgraph-' + self.target + '/bin/diskgraph')
        with self.assertRaises(ValueError):
            self.install(self.package((entry, io.BytesIO())))

    def test_worker_digest_mismatch_refuses_preparation(self):
        with self.assertRaises(ValueError):
            self.install(self.package(bad_worker=True))

    def test_expanded_bytes_have_an_independent_budget(self):
        previous = MODULE.MAX_ARCHIVE_BYTES
        try:
            MODULE.MAX_ARCHIVE_BYTES = 512
            with self.assertRaises(ValueError):
                self.install(self.package())
        finally:
            MODULE.MAX_ARCHIVE_BYTES = previous

    def test_exclusive_write_preserves_existing_active_record(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'slot'
            path.write_bytes(b'DGSL01A\n')
            fd = os.open(directory, os.O_RDONLY)
            try:
                with self.assertRaises(FileExistsError):
                    MODULE.write_at(fd, 'slot', io.BytesIO(b'DGSL01C\n'), 8, 0o600)
                self.assertEqual(path.read_bytes(), b'DGSL01A\n')
            finally:
                os.close(fd)


if __name__ == '__main__':
    unittest.main()
