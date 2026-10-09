from pathlib import Path
import os,subprocess,json,tempfile
root=Path('/workspace/rustwright')
out=root/'target/ci-rollout/guard-probes'
out.mkdir(parents=True,exist_ok=True)
records=[]
for label,mode in [('missing-chrome','missing-chrome'),('missing-firefox','missing-firefox'),('zero-tests','zero'),('ignored-tests','ignored'),('filtered-tests','filtered'),('one-backend-only','one')]:
    with tempfile.TemporaryDirectory(prefix='ci-guard-',dir=out) as directory:
        base=Path(directory)
        binpath=base/'bin'
        binpath.mkdir()
        fake=binpath/'cargo'
        summaries={'zero':'0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out','ignored':'0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out','filtered':'0 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out','one':'1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out'}
        text=summaries.get(mode,summaries['zero'])
        fake.write_text('#!/usr/bin/python3\nimport sys\nif "--version" in sys.argv:print("cargo fake-guard-probe")\nelse:\n print("test chrome_probe ... ok")\n print('+repr('test result: ok. '+text)+' )\n')
        fake.chmod(0o755)
        env=dict(os.environ,PATH=str(binpath)+os.pathsep+os.environ['PATH'],RUSTWRIGHT_CHROME='/usr/bin/true',RUSTWRIGHT_FIREFOX='/usr/bin/true')
        if mode=='missing-chrome':env['RUSTWRIGHT_CHROME']=str(base/'not-installed')
        if mode=='missing-firefox':env.pop('RUSTWRIGHT_FIREFOX',None)
        dest=out/label
        dest.mkdir(exist_ok=True)
        (dest/'report.json').write_text('{"status":"passed","stale":true}')
        result=subprocess.run(['python3',str(root/'scripts/ci/browser_checks.py'),'--output',str(dest)],cwd=root,env=env,capture_output=True,text=True)
        (out/f'{label}.log').write_text(result.stdout+result.stderr)
        report=json.loads((dest/'report.json').read_text())
        assert result.returncode==1,(label,result)
        assert report['status']=='failed' and 'stale' not in report,report
        records.append({'case':label,'exit_code':result.returncode,'expected_failure_verified':True,'error':report['error']})
# Invalid extraction must replace an older README success with a failure record.
path=out/'stale-readme'
path.mkdir(exist_ok=True)
(path/'readme-runtime.json').write_text('{"status":"passed","stale":true}')
result=subprocess.run(['python3',str(root/'scripts/release/readme_runtime.py'),'--output',str(path)],cwd=root,capture_output=True,text=True)
(path/'probe.log').write_text(result.stdout+result.stderr)
report=json.loads((path/'readme-runtime.json').read_text())
assert result.returncode!=0 and report['status']=='failed' and 'stale' not in report
records.append({'case':'stale-readme-summary','exit_code':result.returncode,'expected_failure_verified':True})
(out/'statuses.json').write_text(json.dumps(records,indent=2)+'\n')
print(f'CI/report negative probes: {len(records)}/{len(records)} expected failures verified')
