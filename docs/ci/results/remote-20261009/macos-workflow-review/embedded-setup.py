import hashlib, json, os, pathlib, plistlib, re, subprocess, tempfile
version = os.environ['CHROME_VERSION']
if not re.fullmatch(r'\d+\.\d+\.\d+\.\d+', version):
    raise ValueError(f'Expected the exact installed Chrome version: {version!r}')
platform = {'ARM64': 'mac-arm64', 'X64': 'mac-x64'}[os.environ['RUNNER_ARCH']]
work = pathlib.Path(tempfile.mkdtemp(prefix='rustwright-chrome-', dir=os.environ['RUNNER_TEMP']))
archive = work / 'chrome.zip'
url = f'https://storage.googleapis.com/chrome-for-testing-public/{version}/{platform}/chrome-{platform}.zip'
subprocess.run(['curl', '--fail', '--location', '--silent', '--show-error', '--max-time', '120', '--output', str(archive), url], check=True)
# Re-extract the same engine with its original .app and relative links.
# Copying only .app contents into a toolcache breaks sandbox identity.
subprocess.run(['ditto', '-x', '-k', str(archive), str(work)], check=True)
bundle = work / f'chrome-{platform}' / 'Google Chrome for Testing.app'
executable = bundle / 'Contents/MacOS/Google Chrome for Testing'
if not executable.is_file():
    raise FileNotFoundError(executable)
subprocess.run(['codesign', '--verify', '--deep', '--strict', str(bundle)], check=True)
with (bundle / 'Contents/Info.plist').open('rb') as stream:
    identifier = plistlib.load(stream)['CFBundleIdentifier']
with archive.open('rb') as stream:
    digest = hashlib.file_digest(stream, 'sha256').hexdigest()
links = {str(p.relative_to(bundle)): os.readlink(p) for p in bundle.glob('Contents/Frameworks/*.framework/Versions/Current') if p.is_symlink()}
record = {'version': version, 'url': url, 'archive_sha256': digest, 'bundle_identifier': identifier, 'framework_links': links, 'executable': str(executable), 'signature_verified': True}
output = pathlib.Path('target/ci/chrome-macos-setup.json')
output.parent.mkdir(parents=True, exist_ok=True)
output.write_text(json.dumps(record, indent=2) + '\n', encoding='utf-8')
print(json.dumps(record, indent=2))
subprocess.run([str(executable), '--version'], check=True)
with open(os.environ['GITHUB_OUTPUT'], 'a', encoding='utf-8') as stream:
    stream.write(f'chrome-path={executable}\n')
