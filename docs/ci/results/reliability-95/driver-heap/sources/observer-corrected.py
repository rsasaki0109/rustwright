import sys,os,pathlib,subprocess,json,hashlib,time
root=pathlib.Path('/workspace/rustwright');out=root/'target/reliability95'/sys.argv[1];cycles=int(sys.argv[2]);out.mkdir(parents=True,exist_ok=False)
heap=pathlib.Path('/workspace/.rustwright-env/xvfb/root/usr/bin/heaptrack');printer=heap.with_name('heaptrack_print');binary=root/'target/release/examples/endurance'
def digest(p):return hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()
env=dict(os.environ);env['LD_LIBRARY_PATH']='/workspace/.rustwright-env/xvfb/root/usr/lib/x86_64-linux-gnu';env['RUSTWRIGHT_DRIVER_HEAP_PROFILE']='1'
cmd=[str(heap),'--record-only','-o',str(out/'heaptrack'),str(binary),'firefox',str(cycles),'100','basic']
manifest={'command':cmd,'git_head':subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip(),'binary_sha256':digest(binary),'binary_bytes':binary.stat().st_size,'heaptrack_sha256':digest(heap),'heaptrack_print_sha256':digest(printer),'source_sha256':{str(p.relative_to(root)):digest(p) for p in sorted((root/'crates').rglob('*.rs'))},'endurance_sha256':digest(root/'examples/endurance.rs'),'driver_only':True,'requested_measured':cycles,'requested_warmup':100,'started_unix':time.time()}
(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');records=[]
with (out/'stdout.log').open('w') as raw,(out/'stderr.log').open('w') as err:
 p=subprocess.Popen(cmd,cwd=root,env=env,stdout=subprocess.PIPE,stderr=err,text=True)
 for line in p.stdout:
  raw.write(line);raw.flush()
  if not line.startswith('{'):continue
  r=json.loads(line);records.append(r)
  if r.get('kind')=='metadata':
   driver_maps=pathlib.Path(f"/proc/{r['driver_pid']}/maps").read_text();browser_maps=pathlib.Path(f"/proc/{r['browser_pid']}/maps").read_text()
   verified={'driver_contains_heaptrack':'libheaptrack_preload' in driver_maps,'browser_contains_heaptrack':'libheaptrack_preload' in browser_maps,'browser_binary':str(pathlib.Path(f"/proc/{r['browser_pid']}/exe").resolve()),'browser_pid':r['browser_pid'],'driver_pid':r['driver_pid']}
   verified['browser_binary_sha256']=digest(verified['browser_binary']);(out/'instrumentation.json').write_text(json.dumps(verified,indent=2)+'\n')
   assert verified['driver_contains_heaptrack'] and not verified['browser_contains_heaptrack'],verified
 code=p.wait()
(out/'native.jsonl').write_text(''.join(json.dumps(r)+'\n' for r in records));samples=[r for r in records if r.get('kind')=='sample'];result=next((r for r in reversed(records) if r.get('kind')=='result'),{})
summary={'exit_code':code,'result':result,'samples':samples,'success':code==0 and result.get('success') is True and samples[-1]['completed']==cycles+100}
(out/'summary.json').write_text(json.dumps(summary,indent=2)+'\n');assert summary['success'],summary
trace=next(p for p in out.iterdir() if p.suffix in ('.gz', '.zst'))
analysis=[str(printer),'-f',str(trace),'-l','1','-n','25','-s','5','--disable-builtin-suppressions','--disable-embedded-suppressions','--print-suppressions','1','-M',str(out/'massif.txt')]
with (out/'heaptrack-print.log').open('w') as f:subprocess.run(analysis,env=env,stdout=f,stderr=subprocess.STDOUT,check=True,timeout=120)
text=(out/'heaptrack-print.log').read_text()
import re
allocations=int(re.search(r'calls to allocation functions: (\d+)',text).group(1))
assert allocations > 0, 'Empty heap capture cannot pass'
(out/'profile-validation.json').write_text(json.dumps({'heap_profile_success':True,'calls_to_allocation_functions':allocations,'trace_sha256':digest(trace),'native_workload_success':summary['success']},indent=2)+'\n')
(out/'analysis-command.json').write_text(json.dumps(analysis,indent=2)+'\n')
print(json.dumps({'out':str(out),'success':True,'measured_cycles':cycles,'trace_bytes':trace.stat().st_size}),flush=True)
