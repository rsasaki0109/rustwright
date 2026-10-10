#!/usr/bin/env python3
"""Run finite final native/site checks and retain commands and source identities."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import threading
import time

ROOT = Path('/workspace/rustwright')
BASE = ROOT / 'target/reliability95'
SITES = ['https://example.com/', 'https://developer.mozilla.org/en-US/docs/Web',
         'https://docs.rs/', 'https://jp.mercari.com/', 'https://fril.jp/',
         'https://x.com/', 'https://www.youtube.com/', 'https://www.tiktok.com/',
         'https://www.instagram.com/']

def identity():
    paths = subprocess.check_output(['git', 'ls-files', '-z', 'crates', 'examples', 'tests', 'scripts', '.github', 'Cargo.toml', 'Cargo.lock', 'README.md'], cwd=ROOT).split(b'\0')
    hashes = {p.decode(): hashlib.sha256((ROOT/p.decode()).read_bytes()).hexdigest() for p in paths if p and (ROOT/p.decode()).is_file()}
    return {'head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT,text=True).strip(), 'inputs_sha256':hashes,
            'tracked_input_diff':subprocess.check_output(['git', 'diff', '--stat', '--', 'crates','examples','tests','scripts','.github','Cargo.toml','Cargo.lock','README.md'], cwd=ROOT,text=True)}

def run(name, command, env, timeout, headed=False):
    folder=BASE/'run-records'/name
    folder.mkdir(parents=True,exist_ok=False)
    record={'name':name,'command':command,'source':identity(),'started_unix_ms':round(time.time()*1000),
            'headed':headed,'display':env.get('DISPLAY') if headed else None,
            'display_kind':'owned Xvfb virtual display' if headed else None,
            'selected_browsers':{k:env.get(k) for k in ('RUSTWRIGHT_CHROME','RUSTWRIGHT_FIREFOX')},
            'observer':'xwininfo root tree every 0.5 seconds; no physical display claim'}
    stop=threading.Event()
    def observe():
        with (folder/'native-windows.jsonl').open('w') as f:
            previous=None
            while not stop.is_set():
                try:
                    r=subprocess.run(['xwininfo','-root','-tree'],env=env,capture_output=True,text=True,timeout=3)
                    if r.stdout != previous:
                        windows=[]
                        for line in r.stdout.splitlines():
                            match=re.match(r'\s+(0x[0-9a-fA-F]+).*\("[^"]+" "([^"]+)"\)',line)
                            if match and any(part in match[2].lower() for part in ('chrome','chromium','firefox')):
                                detail=subprocess.run(['xwininfo','-id',match[1]],env=env,capture_output=True,text=True,timeout=3)
                                windows.append({'id':match[1],'class':match[2],'detail':detail.stdout,'exit_code':detail.returncode,'viewable':'Map State: IsViewable' in detail.stdout})
                        f.write(json.dumps({'unix_ms':round(time.time()*1000),'exit_code':r.returncode,'tree':r.stdout,'stderr':r.stderr,'browser_windows':windows})+'\n');f.flush();previous=r.stdout
                except Exception as e:
                    f.write(json.dumps({'observer_error':str(e)})+'\n');f.flush()
                stop.wait(.5)
    observer=threading.Thread(target=observe) if headed else None
    if observer:observer.start()
    try:
        with (folder/'process.log').open('w') as log:
            r=subprocess.run(command,cwd=ROOT,env=env,stdout=log,stderr=subprocess.STDOUT,timeout=timeout)
        record['exit_code']=r.returncode
    except Exception as e:
        record['runner_error']=str(e)
    finally:
        stop.set()
        if observer:observer.join(timeout=5)
        record['ended_unix_ms']=round(time.time()*1000)
        (folder/'run.json').write_text(json.dumps(record,indent=2)+'\n')
    print(json.dumps({'name':name,'exit_code':record.get('exit_code'),'error':record.get('runner_error')}),flush=True)
    return record.get('exit_code',-1)

def main():
    parser=argparse.ArgumentParser();parser.add_argument('kind',choices=['native','sites'])
    args=parser.parse_args();env=dict(os.environ,CARGO_INCREMENTAL='0')
    records=[]
    if args.kind=='native':
        headed=dict(env,DISPLAY=':95',RUSTWRIGHT_CHROME='/workspace/.rustwright-env/chrome155/chromium',RUSTWRIGHT_FIREFOX='/workspace/.rustwright-env/firefox-browser')
        records.append(run('headed-155-157',['python3','scripts/ci/browser_checks.py','--suite','portable','--headed','--output','target/reliability95/native/headed-155-157'],headed,1800,True))
        versions=dict(env,RUSTWRIGHT_CHROME='/workspace/.rustwright-env/chromium',RUSTWRIGHT_FIREFOX='/workspace/.rustwright-env/firefox-esr/firefox-browser')
        records.append(run('headless-151-153',['python3','scripts/ci/browser_checks.py','--suite','portable','--output','target/reliability95/native/headless-151-153'],versions,1800))
    else:
        for backend in ('chrome','firefox'):
            for mode in ('headless','headed'):
                name=f'public-{backend}-{mode}'
                selected=dict(env,DISPLAY=':95',RUSTWRIGHT_CHROME='/workspace/.rustwright-env/chrome155/chromium',RUSTWRIGHT_FIREFOX='/workspace/.rustwright-env/firefox-sites-trusted')
                command=['target/debug/examples/compat_report','--browser',backend,'--'+mode,'--proxy','http://proxy:8080','--timeout-secs','25','--idle-timeout-secs','5','--output',f'target/reliability95/sites/{name}']+SITES
                records.append(run(name,command,selected,1200,mode=='headed'))
    raise SystemExit(0 if all(r==0 for r in records) else 1)

if __name__=='__main__':main()
