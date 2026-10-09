import { readFile, realpath } from 'node:fs/promises';

// The comparison runs on Linux. Reading the identified browser's actual argv
// avoids requiring --enable-automation solely to enable a diagnostic command.
export async function browserLaunchArguments(session, executablePath, io = { readFile, realpath }) {
  const { processInfo } = await session.send('SystemInfo.getProcessInfo');
  const browsers = processInfo.filter(process => process.type === 'browser');
  if (browsers.length !== 1 || !Number.isSafeInteger(browsers[0].id) || browsers[0].id <= 0) {
    throw new Error('browser process identity is ambiguous or invalid');
  }
  const pid = browsers[0].id;
  const commandLine = await io.readFile(`/proc/${pid}/cmdline`, 'utf8');
  if (!commandLine.endsWith('\0')) throw new Error('browser argv is missing or truncated');
  const args = commandLine.slice(0, -1).split('\0');
  const [actual, expected] = await Promise.all([
    io.realpath(`/proc/${pid}/exe`), io.realpath(executablePath),
  ]);
  if (actual !== expected || !args[0]) throw new Error('browser executable identity mismatch');
  return args;
}
