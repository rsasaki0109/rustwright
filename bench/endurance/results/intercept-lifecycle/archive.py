import datetime
import hashlib
import json
import platform
import re
import shutil
import subprocess
import tarfile
from collections import Counter
from pathlib import Path

ROOT = Path('/workspace/rustwright')
RUN = ROOT / 'target/intercept-lifecycle'
OUT = ROOT / 'bench/endurance/results/intercept-lifecycle'

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def save(name, data):
    (OUT / name).write_text(json.dumps(data, indent=2, ensure_ascii=False) + '\n')

def summarize(path):
    rows = re.findall(r'test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out', path.read_text())
    return {'suites': len(rows), 'passed': sum(int(x[1]) for x in rows), 'failed': sum(int(x[2]) for x in rows), 'ignored': sum(int(x[3]) for x in rows), 'filtered': sum(int(x[5]) for x in rows), 'all_reported_suites_ok': bool(rows) and all(x[0] == 'ok' for x in rows)}

def records(path):
    result=[]
    for line in path.read_text().splitlines():
        pos=line.find('{')
        if pos < 0:
            continue
        try:
            value=json.loads(line[pos:])
        except json.JSONDecodeError:
            continue
        if isinstance(value,dict) and 'cycle' in value:
            result.append(value)
    return result

def archive_tree(base, output):
    files=sorted(p.relative_to(base).as_posix() for p in base.rglob('*') if p.is_file() and p.suffix in {'.rs','.toml','.lock','.js','.html','.css'})
    hashes={name:sha(base/name) for name in files}
    save(output+'-source-sha256.json',hashes)
    with tarfile.open(OUT/(output+'-source.tar.gz'),'w:gz') as archive:
        for name in files:
            archive.add(base/name,arcname=name)
    return {'file_count':len(files),'archive_sha256':sha(OUT/(output+'-source.tar.gz'))}

OUT.mkdir(parents=True,exist_ok=True)
frozen=json.loads((OUT/'source-sha256.json').read_text())
paths=subprocess.check_output(['rg','--files','--hidden','-g','!.git'],cwd=ROOT,text=True).splitlines()
paths=sorted(name for name in paths if Path(name).suffix in {'.rs','.toml','.lock','.py','.mjs','.js','.yml','.yaml','.html','.css'} and '/results/' not in name)
current={name:sha(ROOT/name) for name in paths}
check={'unchanged':current==frozen,'changed_or_added':sorted(name for name in current if current[name]!=frozen.get(name)),'missing':sorted(set(frozen)-set(current))}
save('source-check.json',check)
assert check['unchanged'],check

for name in ['unit','native','msrv','clippy','fmt','whitespace']:
    shutil.copyfile(RUN/'final'/(name+'.log'),OUT/(name+'-final.log'))
for source in RUN.iterdir():
    if source.is_file() and source.name not in {'baseline-source.tar.gz','baseline-source-sha256.json'}:
        shutil.copyfile(source,OUT/source.name)
shutil.copytree(RUN/'native',OUT/'native',dirs_exist_ok=True)
shutil.copyfile(ROOT/'bench/endurance/INTERCEPTION_LIFECYCLE.md',OUT/'REPORT.md')
shutil.copyfile(Path(__file__),OUT/'archive.py')

baseline=archive_tree(RUN/'baseline-workspace','baseline')
identical=archive_tree(RUN/'identical-after-workspace','identical-after')
initial=OUT/'initial-tests-before.rs'
assert sha(initial)==sha(RUN/'baseline-workspace/crates/rustwright-bidi/src/interception_tests.rs')
assert sha(initial)==sha(RUN/'identical-after-workspace/crates/rustwright-bidi/src/interception_tests.rs')
for name in ['browser.rs','connection.rs','session.rs','network.rs','lib.rs','interception.rs']:
    assert sha(ROOT/'crates/rustwright-bidi/src'/name)==sha(RUN/'identical-after-workspace/crates/rustwright-bidi/src'/name)

binary_paths=sorted(set(match for name in ['unit-final.log','native-final.log'] for match in re.findall(r'\((target/debug/deps/[^)]+)\)',(OUT/name).read_text())))
binaries={name:sha(ROOT/name) for name in binary_paths}
staged_binaries={}
for name in ['before-test.log','identical-after.log','native/before.log']:
    for match in re.findall(r'\(([^)]+/debug/deps/[^)]+)\)',(OUT/name).read_text()):
        path=Path(match)
        if not path.is_absolute():
            path=ROOT/path
        staged_binaries[match]=sha(path)

all_rows=records(OUT/'native-final.log')
intercepts=[r for r in all_rows if 'known_intercept_ids' in r]
(OUT/'intercept-checkpoints.jsonl').write_text(''.join(json.dumps(r,ensure_ascii=False)+'\n' for r in intercepts))
counts=dict(Counter(r['mode'] for r in intercepts))
assert len(intercepts)==71,counts
assert sum(r['mode']!='request-response-child-frame' for r in intercepts)==70
assert all(r['known_intercepts_absent'] and r['pending']==0 for r in intercepts)
native_before=summarize(OUT/'native/before.log')
unit_before=summarize(OUT/'before-test.log')
identical_after=summarize(OUT/'identical-after.log')
unit=summarize(OUT/'unit-final.log')
native=summarize(OUT/'native-final.log')
assert unit['passed']==158 and unit['all_reported_suites_ok'] and unit['failed']==0,unit
assert native['passed']==160 and native['suites']==12 and native['all_reported_suites_ok'] and native['failed']==0,native
assert native_before['passed']==0 and native_before['failed']==4,native_before
assert unit_before['passed']==1 and unit_before['failed']==9,unit_before
assert identical_after['passed']==10 and identical_after['failed']==0,identical_after
for name in ['msrv-final.log','clippy-final.log']:
    data=(OUT/name).read_text()
    assert 'Finished' in data and '\nerror' not in data,(name,data)
assert not (OUT/'fmt-final.log').read_text()
assert not (OUT/'whitespace-final.log').read_text()

manifest={
    'recorded_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),
    'git_head':subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),
    'git_status':subprocess.check_output(['git','status','--short'],cwd=ROOT,text=True).splitlines(),
    'platform':platform.platform(),
    'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),
    'source_file_count':len(frozen),
    'source_scope':'nonignored Rust/Cargo/JS/Python/HTML/CSS/YAML code and configuration excluding historical results; final native/Clippy/MSRV checks use this source; workspace library sources unchanged by subsequent fixture whitespace-only Clippy correction',
    'source_archive_sha256':sha(OUT/'source.tar.gz'),
    'binary_sha256':binaries,
    'staged_binary_sha256':staged_binaries,
    'baseline':baseline,
    'identical_after':identical,
    'initial_test_source_sha256':sha(initial),
    'same_initial_test_source_both_stages':True,
    'same_initial_after_production_files_as_final':True,
    'native_fixture_difference':'Four native test functions byte-identical between baseline and final; support fixture differs only by whitespace/newlines resolving Clippy possible_missing_else. Earlier three-case and interrupted initial runs are preserved separately.',
    'commands':{
        'unit':'cargo test --offline --locked --workspace --lib -- --test-threads=1',
        'native':'cargo test --offline --locked --no-fail-fast -p rustwright-integration-tests --test http_creation --test http_compat --test http_contexts --test http_disconnect --test http_frames --test http_navigation --test http_network_idle --test http_actionability --test http_clipped_control --test http_shutdown --test http_frame_helpers --test http_intercept_lifecycle -- --nocapture --test-threads=1',
        'msrv':'cargo +1.85.0 check --offline --locked --workspace --all-targets --target-dir /workspace/.rustwright-env/target-msrv',
        'clippy':'cargo clippy --offline --locked --workspace --all-targets -- -D warnings',
        'format':'cargo fmt --all -- --check',
        'whitespace':'git diff --check',
        'identical_initial_after':'cargo test --offline --locked --manifest-path target/intercept-lifecycle/identical-after-workspace/Cargo.toml --target-dir target/intercept-lifecycle/identical-target -p rustwright-bidi --lib browser::interception_tests -- --test-threads=1',
        'native_baseline':'cargo test --offline --locked --manifest-path target/intercept-lifecycle/baseline-workspace/Cargo.toml --target-dir target/intercept-lifecycle/baseline-target -p rustwright-integration-tests --test http_intercept_lifecycle -- --test-threads=1 --nocapture',
    },
    'scope':'headless Linux/local HTTP interception lifetime. Known allocated IDs independently queried absent; no complete remote census, heap reachability, performance repeat, Windows/macOS/headed/real-site/file-data URL/remote-CI evidence',
}
assert sha(ROOT/'tests/http_intercept_lifecycle.rs')==sha(RUN/'baseline-workspace/tests/http_intercept_lifecycle.rs')
save('manifest.json',manifest)
validation={'unit':unit,'native':native,'initial_protocol_before':unit_before,'identical_initial_protocol_after':identical_after,'native_before':native_before,'intercept_checkpoints':{'total':len(intercepts),'lifecycle':70,'by_mode':counts,'versions':sorted(set(str(r['version']) for r in intercepts)),'all_known_ids_absent':True,'all_pending_zero':True},'source_unchanged':True,'clippy':'passed','msrv':'passed: Rust 1.85.0 all targets','format':'passed','whitespace':'passed'}
save('validation.json',validation)
print(json.dumps(validation,ensure_ascii=False))
