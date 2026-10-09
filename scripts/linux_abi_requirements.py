"""核对最终GNU ELF的glibc版本需求；不修补镜像、不推定内核能力或原生验收。"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import struct
from worker_manifest import admitted_file

MAX_IMAGE_BYTES = 128 * 1024 * 1024
GNU_TARGET_MACHINES = {'x86_64-unknown-linux-gnu': 62, 'aarch64-unknown-linux-gnu': 183}


def glibc_requirements(raw):
    """参数：有限原始ELF字节；返回：数值排序的glibc版本，未知/损坏格式拒绝。"""
    if len(raw) < 64 or len(raw) > MAX_IMAGE_BYTES or raw[:7] != b'\x7fELF\x02\x01\x01':
        raise ValueError('expected bounded native64 little-endian ELF')
    if struct.unpack_from('<H', raw, 16)[0] not in (2, 3):
        raise ValueError('GNU package requires an executable or dynamic ELF object')
    if struct.unpack_from('<H', raw, 18)[0] not in (62, 183):
        raise ValueError('unsupported GNU package architecture')

    def bounded(offset, length):
        if offset < 0 or length < 0 or offset + length > len(raw):
            raise ValueError('ELF metadata exceeds image bounds')
        return raw[offset:offset + length]

    def string(table, offset):
        if not 0 <= offset < len(table):
            raise ValueError('invalid ELF string offset')
        end = table.find(b'\0', offset, min(len(table), offset + 256))
        if end < 0:
            raise ValueError('unterminated or oversized ELF metadata name')
        try:
            return table[offset:end].decode('ascii')
        except UnicodeDecodeError as error:
            raise ValueError('ELF metadata name is not ASCII') from error

    offset = struct.unpack_from('<Q', raw, 40)[0]
    width, count, names_index = struct.unpack_from('<HHH', raw, 58)
    if width != 64 or not 1 <= count <= 4096 or not 0 < names_index < count:
        raise ValueError('unsupported ELF section table')
    table = bounded(offset, width * count)
    sections = [struct.unpack_from('<IIQQQQIIQQ', table, i * width) for i in range(count)]
    names_section = sections[names_index]
    names = bounded(names_section[4], names_section[5])
    by_name = {}
    for section in sections[1:]:
        name = string(names, section[0])
        if name in by_name:
            raise ValueError('duplicate ELF section name')
        by_name[name] = section
    needs = by_name.get('.gnu.version_r')
    if needs is None or needs[1] != 0x6ffffffe or not 0 < needs[6] < count:
        raise ValueError('GNU package has no verifiable version requirements')
    strings_section = sections[needs[6]]
    if strings_section[1] != 3:
        raise ValueError('version requirements do not reference a string table')
    strings = bounded(strings_section[4], strings_section[5])
    data = bounded(needs[4], needs[5])
    position = 0
    versions = set()
    records = 0
    while True:
        if position + 16 > len(data) or records >= 512:
            raise ValueError('invalid or oversized ELF version requirements')
        version, auxiliaries, library, auxiliary, next_record = struct.unpack_from('<HHIII', data, position)
        if version != 1 or not 1 <= auxiliaries <= 512 or auxiliary < 16:
            raise ValueError('invalid GNU version record')
        string(strings, library)
        cursor = position + auxiliary
        for index in range(auxiliaries):
            if cursor + 16 > len(data) or records >= 512:
                raise ValueError('invalid or oversized GNU version auxiliary')
            _, _, _, name, next_auxiliary = struct.unpack_from('<IHHII', data, cursor)
            text = string(strings, name)
            if text.startswith('GLIBC_'):
                numeric = text[6:]
                if not re.fullmatch(r'[0-9]+(?:\.[0-9]+)+', numeric):
                    raise ValueError('unknown glibc ABI requirement')
                versions.add(tuple(map(int, numeric.split('.'))))
            records += 1
            if index + 1 < auxiliaries:
                if next_auxiliary < 16:
                    raise ValueError('invalid GNU version auxiliary chain')
                cursor += next_auxiliary
            elif next_auxiliary != 0:
                raise ValueError('GNU version auxiliary count mismatch')
        if next_record == 0:
            break
        if next_record < 16:
            raise ValueError('invalid GNU version record chain')
        position += next_record
    if not versions:
        raise ValueError('GNU package has no glibc compatibility proof')
    return ['.'.join(map(str, version)) for version in sorted(versions)]


def require_baseline(raw, maximum):
    """参数：原镜像及声明基线；返回：版本需求，超过声明直接拒绝。"""
    if not re.fullmatch(r'[0-9]+(?:\.[0-9]+)+', maximum):
        raise ValueError('invalid declared glibc baseline')
    versions = glibc_requirements(raw)
    if tuple(map(int, versions[-1].split('.'))) > tuple(map(int, maximum.split('.'))):
        raise ValueError(f'glibc {versions[-1]} exceeds advertised baseline {maximum}')
    return versions


def inspect_image(path, maximum, expected_machine=None):
    """参数：真实有限文件、既定基线和可选机器码；返回：同句柄读取的静态ABI及摘要。"""
    path = Path(path)
    # 复用已有不跟随链接、长度预算与打开身份复验，不以另一遍摘要拼接证明。
    with admitted_file(path, MAX_IMAGE_BYTES) as (stream, before):
        raw = stream.read(before.st_size)
        if len(raw) != before.st_size:
            raise ValueError('ELF image changed during bounded read')
        versions = require_baseline(raw, maximum)
        if expected_machine is not None and struct.unpack_from('<H', raw, 18)[0] != expected_machine:
            raise ValueError('ELF architecture differs from package target')
        result = {'name': path.name, 'bytes': len(raw), 'sha256': hashlib.sha256(raw).hexdigest(),
                  'glibc_requirements': versions}
    return result


def require_gnu_package(bin_dir, target):
    """参数：最终三个GNU镜像目录及目标；返回：固定2.17声明的静态证据，不替代旧环境运行。"""
    if target not in GNU_TARGET_MACHINES:
        raise ValueError('unsupported GNU package target')
    images = [inspect_image(Path(bin_dir) / name, '2.17', GNU_TARGET_MACHINES[target])
              for name in ('diskgraph', 'diskgraph-mcp', 'diskgraph-scan-worker')]
    return {'target': target, 'advertised_baseline': '2.17', 'images': images,
            'runtime_qualification': False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--maximum-glibc', required=True)
    parser.add_argument('--target', choices=sorted(GNU_TARGET_MACHINES))
    parser.add_argument('images', nargs='+', type=Path)
    arguments = parser.parse_args()
    results = []
    for path in arguments.images:
        try:
            result = inspect_image(path, arguments.maximum_glibc,
                                   GNU_TARGET_MACHINES.get(arguments.target))
        except (OSError, ValueError) as error:
            # 失败绑定实际输入，不让先前已通过的镜像输出被误当成完整包资格。
            # 原文件准入、架构及基线判定不变，不吞掉内部编程错误或取消。
            report = {
                'ok': False, 'error': 'gnu_abi_refused', 'image': str(path),
                'target': arguments.target,
                'advertised_baseline': arguments.maximum_glibc,
                'detail': str(error),
            }
            message = json.dumps(report)
            if len(message) + 1 > 65536:
                # json.dumps 默认 ASCII 转义，字符数就是最终 UTF-8 字节数。
                # 截断展示字段但保留完整原诊断摘要，不构造伪造的完整资格报告。
                report['diagnostic_sha256'] = hashlib.sha256(message.encode('ascii')).hexdigest()
                report['diagnostic_truncated'] = True
                report['image'] = report['image'][:4096]
                report['advertised_baseline'] = report['advertised_baseline'][:128]
                report['detail'] = report['detail'][:2048]
                message = json.dumps(report)
            parser.exit(1, message + '\n')
        result.update(image=path.name, advertised_baseline=arguments.maximum_glibc)
        results.append(result)
    # 全部有限镜像已完成同句柄检查后才发布原格式的逐行成功报告。
    for result in results:
        print(json.dumps(result))


if __name__ == '__main__':
    main()
