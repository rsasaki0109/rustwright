import { test } from 'node:test';
import assert from 'node:assert/strict';
import { browserLaunchArguments } from './launch_metadata.mjs';

function fixture(processInfo = [{ type: 'browser', id: 123 }], commandLine = '/chrome\0--remote-debugging-pipe\0') {
  return {
    session: { async send(method) { assert.equal(method, 'SystemInfo.getProcessInfo'); return { processInfo }; } },
    io: {
      async readFile(path, encoding) { assert.equal(path, '/proc/123/cmdline'); assert.equal(encoding, 'utf8'); return commandLine; },
      async realpath(path) { assert.ok(['/proc/123/exe', '/chosen/chrome'].includes(path)); return '/chrome'; },
    },
  };
}
test('reads identified browser argv without changing launch flags', async () => {
  const { session, io } = fixture([{ type: 'renderer', id: 456 }, { type: 'browser', id: 123 }]);
  assert.deepEqual(await browserLaunchArguments(session, '/chosen/chrome', io), ['/chrome', '--remote-debugging-pipe']);
});
test('rejects absent, ambiguous or invalid browser PID before filesystem reads', async () => {
  for (const processes of [[], [{ type: 'browser', id: 0 }], [{ type: 'browser', id: 1.5 }], [{ type: 'browser', id: '123' }], [{ type: 'browser', id: 123 }, { type: 'browser', id: 124 }]]) {
    const { session, io } = fixture(processes);
    io.readFile = async () => assert.fail('invalid PID must not be read');
    await assert.rejects(browserLaunchArguments(session, '/chosen/chrome', io), /identity/);
  }
});
test('rejects truncated or empty argv', async () => {
  for (const commandLine of ['', '/chrome', '\0']) {
    const { session, io } = fixture(undefined, commandLine);
    await assert.rejects(browserLaunchArguments(session, '/chosen/chrome', io), /argv|identity/);
  }
});
test('rejects an executable mismatch rather than attributing another process', async () => {
  const { session, io } = fixture();
  io.realpath = async path => path === '/proc/123/exe' ? '/other/chrome' : '/chrome';
  await assert.rejects(browserLaunchArguments(session, '/chosen/chrome', io), /identity mismatch/);
});
test('propagates inaccessible process metadata', async () => {
  const { session, io } = fixture();
  io.readFile = async () => { throw new Error('process exited'); };
  await assert.rejects(browserLaunchArguments(session, '/chosen/chrome', io), /process exited/);
});
