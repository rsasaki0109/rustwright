import pathlib,json,re,hashlib,collections
ROOT=pathlib.Path(__file__).resolve().parent;EXT=ROOT/'artifacts/headed-versions-sites'
report=json.loads((EXT/'report.json').read_text());assert report['status']=='passed'
commands={c['name']:c for c in report['commands']};assert len(commands)==9
VERSIONS={'chrome_current':'155.0.8059.39','firefox_current':'157.0.1','chrome_previous':'151.0.7922.138','firefox_esr':'153.4.0esr'}
URLS=['https://example.com/','https://developer.mozilla.org/en-US/docs/Web','https://docs.rs/']
def matches(text,version):return bool(re.search(r'(?<![0-9A-Za-z_.])'+re.escape(version)+r'(?![0-9A-Za-z_.])',text))
for key,version in VERSIONS.items():
 text=(EXT/('version-'+key+'.log')).read_text();assert matches(text,version),(key,text);assert report['browsers'][key]['version_output']==text.strip()
assert set(commands)=={'version-'+key for key in VERSIONS}|{'headed-current','headless-alternate','build-compat-report','sites-chrome','sites-firefox'}
for c in commands.values():assert c['status']=='passed' and c['exit_code']==0 and not c.get('timed_out') and not c.get('parent_error'),c
counts={};proofs={};pngs=[];sites={}
for name,headed in [('headed-current',True),('headless-alternate',False)]:
 native=json.loads((EXT/name/'report.json').read_text());assert native['status']=='passed' and native['passed']==79 and native['headless']==(not headed);targets={};backends=collections.Counter()
 for case in native['cases']:
  text=re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]','',(EXT/name/(case['target']+'.log')).read_text());summary=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; \d+ measured; (\d+) filtered out;',text);assert len(summary)==1;values=list(map(int,summary[0]));assert values==[case['passed'],0,0,0] and case['exit_code']==0;names=re.findall(r'^test ((?:chrome|firefox)_[^ ]+) \.\.\.',text,re.M);assert len(names)==case['passed'];backends.update(n.split('_',1)[0] for n in names);targets[case['target']]=case['passed']
 assert sum(targets.values())==79
 for backend,key in [('Chrome','chrome_current' if headed else 'chrome_previous'),('Firefox','firefox_current' if headed else 'firefox_esr')]:
  observations=commands[name]['protocol_version_observations'][backend];assert observations
  for observed in observations:
   raw=(EXT/name/(observed['target']+'.log')).read_text();assert observed['line'] in raw.splitlines();assert matches(observed['line'],VERSIONS[key]) or matches(observed['line'],VERSIONS[key].removesuffix('esr'))
 counts[name]={'passed':79,'failed':0,'ignored':0,'filtered':0,'cases_by_target':targets,'cases_by_backend':dict(backends),'mode':native['mode'],'browser_bindings':native['browsers'],'protocol_version_observations':commands[name]['protocol_version_observations']}
for command_name,backends in [('headed-current',['chrome','firefox']),('sites-chrome',['chrome']),('sites-firefox',['firefox'])]:
 c=commands[command_name];assert not c['window_observer_error'];raw=EXT/(command_name+'-windows.jsonl');records=[json.loads(line) for line in raw.read_text().splitlines()];proofs[command_name]={}
 for backend in backends:
  proof=c['mapped_windows'][backend];assert proof['map_state']=='IsViewable';window=proof['window'];pid=proof['pid'];assert proof['command_ancestor_pid']>0
  props=[r for r in records if r['command']==['xprop','-id',window,'WM_CLASS','_NET_WM_PID'] and r['exit_code']==0];stats=[r for r in records if r['command']==['xwininfo','-id',window,'-stats'] and r['exit_code']==0 and re.search(r'Map State:\s+IsViewable\b',r['stdout'])];assert stats and any(re.search(r'_NET_WM_PID\(CARDINAL\)\s*=\s*'+str(pid)+r'\b',r['stdout']) for r in props)
  proofs[command_name][backend]={'proof':proof,'raw_x11_property_and_map_records_match':True,'ownership_scope':'Live PID descendant verified by exact-source helper during observation; ancestry cannot be recreated after runner exit.'}
for backend in ['chrome','firefox']:
 d=EXT/('sites-'+backend);site_report=json.loads((d/'report.json').read_text());assert site_report['status']=='completed' and site_report['headed_requested'] is True and site_report['proxy'] is None;assert all(op['status']=='passed' for op in site_report['operations']);observations=[]
 for site in site_report['sites']:
  assert site['operations_succeeded'] is True and all(op['status']=='passed' for op in site['operations']);path=d/pathlib.Path(site['screenshot']).name;data=path.read_bytes();assert data.startswith(b'\x89PNG\r\n\x1a\n') and len(data)>100;digest=hashlib.sha256(data).hexdigest();claimed=[v for v in commands['sites-'+backend]['screenshots'] if pathlib.Path(v['path']).name==path.name];assert len(claimed)==1 and claimed[0]['sha256']==digest
  pngs.append({'backend':backend,'path':str(path.relative_to(ROOT)),'sha256':digest,'bytes':len(data)})
  observations.append({k:site.get(k) for k in ['requested_url','final_url','title','observation','operations_succeeded','observed_document_statuses','document_status_attribution','document_status_selection']})
 assert len(observations)==3 and [o['requested_url'] for o in observations]==URLS;assert matches(str(site_report['browser']['version']),VERSIONS[backend+'_current']);sites[backend]={'browser':site_report['browser'],'certificate_policy':site_report['certificate_policy'],'proxy':site_report['proxy'],'root_operations_succeeded':True,'site_observations':observations,'http_access_denied_observations':sum(x['observation']=='http_access_denied' for x in observations),'http_error_observations':sum(any(status>=400 for status in x['observed_document_statuses']) for x in observations)}
value={'status':'passed','exact_binary_versions':report['browsers'],'native_suites':counts,'mapped_window_proofs':proofs,'public_sites':sites,'screenshots':pngs,'scope':'Native window proof per headed command, not each79-case window. HTTP denial/error observations stay separate from mandatory successful operations. Default browser TLS; no explicit proxy or cert bypass. Static public reads only.'};(ROOT/'extended-verification.json').write_text(json.dumps(value,indent=2)+'\n');print(json.dumps({'native':counts,'sites':sites,'screenshots':len(pngs)}))
