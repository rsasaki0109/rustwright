#!/usr/bin/env python3
"""Owned synthetic WebSocket/CDP process; not a browser or renderer."""
import base64
import hashlib
import json
import os
from pathlib import Path
import signal
import socket
import struct
import sys
import time
import urllib.request

mode = os.environ['CONTROL_MODE']
Path(os.environ['CONTROL_PID']).write_text(str(os.getpid()))
signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
if mode in ('early-exit', 'known-sandbox-error'):
    print('No usable sandbox!' if mode == 'known-sandbox-error' else 'controlled early exit', file=sys.stderr, flush=True)
    sys.exit(23)
if mode == 'missing-endpoint':
    time.sleep(60)
    sys.exit(0)
profile = Path(next(arg.split('=', 1)[1] for arg in sys.argv if arg.startswith('--user-data-dir=')))
server = socket.socket()
server.bind(('127.0.0.1', 0))
server.listen(1)
(profile / 'DevToolsActivePort').write_text(str(server.getsockname()[1]) + '\n/devtools/browser/controlled-peer\n')
connection, _ = server.accept()
header = b''
while not header.endswith(b'\r\n\r\n'):
    header += connection.recv(1)
headers = dict(line.split(':', 1) for line in header.decode().split('\r\n')[1:] if ':' in line)
key = headers['Sec-WebSocket-Key'].strip()
accept = base64.b64encode(hashlib.sha1((key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest()).decode()
if mode == 'bad-handshake':
    accept = 'incorrect-accept'
connection.sendall(('HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: keep-alive, Upgrade\r\nSec-WebSocket-Accept: ' + accept + '\r\n\r\n').encode())


def read(size):
    result = b''
    while len(result) < size:
        data = connection.recv(size - len(result))
        if not data:
            sys.exit(0)
        result += data
    return result


def request():
    while True:
        first, second = read(2)
        assert second & 0x80, 'client frames must be masked'
        length = second & 0x7f
        if length == 126:
            length = struct.unpack('!H', read(2))[0]
        elif length == 127:
            length = struct.unpack('!Q', read(8))[0]
        mask = read(4)
        payload = bytes(value ^ mask[index % 4] for index, value in enumerate(read(length)))
        if first & 0x0f == 10:
            assert payload == b'ping'
            continue
        return json.loads(payload)


def frame(opcode, payload, final=True):
    length = len(payload)
    prefix = bytes([(0x80 if final else 0) | opcode])
    prefix += bytes([length]) if length < 126 else bytes([126]) + struct.pack('!H', length) if length < 65536 else bytes([127]) + struct.pack('!Q', length)
    connection.sendall(prefix + payload)


def send(value):
    payload = json.dumps(value).encode()
    if mode == 'fragmented-ping':
        frame(9, b'ping')
        split = len(payload) // 2
        frame(1, payload[:split], False)
        frame(0, payload[split:])
    else:
        frame(1, payload)


url = None
while True:
    message = request()
    print(json.dumps({'received': message}), flush=True)
    method = message['method']
    result = {}
    if mode == 'close-frame':
        frame(8, struct.pack('!H', 1000))
        time.sleep(60)
    if mode == 'oversized-frame':
        connection.sendall(bytes([0x81, 127]) + struct.pack('!Q', 1024 * 1024 + 1))
        time.sleep(60)
    if mode == 'masked-server':
        connection.sendall(bytes([0x81, 0x80]))
        time.sleep(60)
    if mode == 'bad-continuation':
        frame(0, b'{}')
        time.sleep(60)
    if method == 'Target.createTarget':
        result = {'targetId': 'target-main'}
    elif method == 'Target.attachToTarget':
        result = {'sessionId': 'session-main'}
    elif method == 'Page.navigate':
        url = message['params']['url']
        urllib.request.urlopen(url, timeout=2).read()
        session = 'foreign-session' if mode == 'foreign-session-event' else 'session-main'
        lifecycle = {'frameId': 'foreign-frame' if mode == 'foreign-frame' else 'frame-main', 'loaderId': 'foreign-loader' if mode == 'foreign-loader' else 'loader-main', 'name': 'DOMContentLoaded'}
        send({'method': 'Page.lifecycleEvent', 'sessionId': session, 'params': lifecycle})
        send({'method': 'Network.responseReceived', 'sessionId': 'session-main', 'params': {'frameId': 'frame-main', 'loaderId': 'loader-main', 'type': 'Document', 'requestId': 'request-main', 'response': {'url': url, 'status': 403 if mode == 'non200' else 200, 'mimeType': 'text/html'}}})
        send({'method': 'Network.loadingFinished', 'sessionId': 'session-main', 'params': {'requestId': 'request-main'}})
        result = {'frameId': 'frame-main', 'loaderId': 'loader-main'}
    elif method == 'Runtime.evaluate':
        result = {'result': {'type': 'object', 'value': {'url': url, 'title': 'wrong title' if mode == 'bad-dom' else 'Rustwright Linux sandbox preflight', 'body': '<h1>ready</h1>', 'readyState': 'complete'}}}
    elif method == 'Browser.close':
        if mode == 'close-hang':
            time.sleep(60)
        if mode == 'close-without-ack':
            sys.exit(0)
    reply = {'id': message['id'] + (1 if mode == 'wrong-id' else 0), 'result': result}
    if message.get('sessionId'):
        reply['sessionId'] = 'foreign-session' if mode == 'wrong-session' else message['sessionId']
    send(reply)
    if method == 'Browser.close':
        sys.exit(0)
