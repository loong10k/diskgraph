"""隔离原生协议夹具客户端：生成真实Linux时钟材料，不是产品认证实现。"""
import json
import os
import socket
import struct
import sys
import time


def request(mode):
    """共享真实期限的正负请求；每个负例只改变待拒绝的查询字段。"""
    namespace = os.stat('/proc/self/ns/time')
    query = {'command': 'doctor', 'subcommand': None, 'options': [], 'switches': []}
    body = {'deadline': {'version': 1, 'domain': [1, namespace.st_dev, namespace.st_ino],
                         'expires_nanos': time.monotonic_ns() + 25_000_000_000}, 'query': query}
    if mode == 'override':
        query['options'] = [['principal', 'administrator']]
    elif mode == 'unknown_command':
        query['command'] = 'arbitrary'
    elif mode == 'duplicate':
        query.update(command='node', options=[['scope', 'a'], ['scope', 'b']])
    elif mode == 'bad_numeric':
        query.update(command='top', options=[['scope', 'a'], ['limit', '-1']])
    elif mode == 'write':
        query.update(command='scope', subcommand='remove', options=[['scope', 'a']])
    elif mode == 'unknown_field':
        body['environment'] = {'DISKGRAPH_LINUX_SUPERVISOR_FD': '5'}
    elif mode not in ('doctor', 'disconnect'):
        raise ValueError('unknown fixed qualification case')
    return b'DG_FRONT_QUERY_V1\n' + json.dumps(body).encode('utf-8')


def remaining(original_deadline):
    value = original_deadline - time.monotonic()
    if value <= 0:
        raise TimeoutError("original public deadline expired")
    return value


def exact(connection, length, original_deadline):
    result = bytearray()
    while len(result) < length:
        connection.settimeout(remaining(original_deadline))
        part = connection.recv(length - len(result))
        if not part:
            raise AssertionError('truncated original frame')
        result.extend(part)
    return result


def main():
    path, mode = sys.argv[1:]
    original_deadline = time.monotonic() + 30
    output = bytearray()
    errors = bytearray()
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        # 不刷新每片期限；socket timeout每次由同一绝对期限扣除。
        connection.settimeout(remaining(original_deadline))
        connection.connect(path)
        if mode == 'stop_before_query':
            assert connection.recv(1) == b'', 'stopped admission must close original public connection'
            print(json.dumps({'mode': mode, 'public_eof': True, 'request_sent': False}))
            return
        connection.sendall(request(mode))
        connection.shutdown(socket.SHUT_WR)
        if mode == 'disconnect':
            print(json.dumps({'mode': mode, 'public_disconnected': True,
                              'private_completion_injected': False}))
            return
        while True:
            connection.settimeout(remaining(original_deadline))
            kind, size = struct.unpack('>BI', exact(connection, 5, original_deadline))
            assert kind in (1, 2, 3) and 0 < size <= 4096
            assert kind != 3 or size == 4
            connection.settimeout(remaining(original_deadline))
            frame = exact(connection, size, original_deadline)
            if kind == 3:
                code = struct.unpack('>i', frame)[0]
                connection.settimeout(remaining(original_deadline))
                assert connection.recv(1) == b'', 'data after original terminal'
                break
            (output if kind == 1 else errors).extend(frame)
            assert len(output) + len(errors) <= 1 << 20
    assert time.monotonic() < original_deadline
    if mode == 'doctor':
        assert code == 0 and json.loads(output)['ok'] is True
    else:
        assert code == 10 and not output and not errors, (code, output, errors)
    print(json.dumps({'mode': mode, 'status': code, 'original_public_eof': True,
                      'valid_linux_clock': True, 'public_output_bytes': len(output)}))


if __name__ == '__main__':
    main()
