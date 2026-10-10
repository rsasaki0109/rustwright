import json,hashlib,shutil
from pathlib import Path
from urllib.parse import urlparse
OUT=Path(__file__).resolve().parent
reports=[]
for name in ['run1','run2','run3']:
 rows=[json.loads(line) for line in (OUT/name/'events.jsonl').read_text().splitlines()]
 def events(url):return [o for o in rows if o['kind']=='event' and o['event']['method'].startswith('network.') and o['event']['params']['request']['url'].endswith(url)]
 def methods(es):return [e['event']['method'] for e in es]
 def bymethod(es,m):return [e for e in es if e['event']['method']==m]
 checks=[]
 def record(what,**details):checks.append({'observation':what,'validated':True,**details})
 before='network.beforeRequestSent';started='network.responseStarted';completed='network.responseCompleted';error='network.fetchError'
 cap=next(o['value'] for o in rows if o['kind']=='capabilities')
 assert cap['capabilities']['browserName']=='firefox'
 if name!='run3':
  stream=events('/slow?top'); assert len(bymethod(stream,completed))==1
  gap=bymethod(stream,completed)[0]['t']-bymethod(stream,started)[0]['t'];assert gap>.7
  record('response completion follows delayed body, not headers',header_to_completion_seconds=gap)
  redirects=events('/redirect1')+events('/redirect2')+events('/slow?redirect');redirects.sort(key=lambda o:o['t'])
  starts=bymethod(redirects,before);finishes=bymethod(redirects,completed)
  assert [e['event']['params']['redirectCount'] for e in starts]==[0,1,2]
  assert [e['event']['params']['redirectCount'] for e in finishes]==[0,1,2]
  assert len({e['event']['params']['request']['request'] for e in redirects})==1
  assert [e['event']['params']['response']['status'] for e in finishes]==[302,307,200]
  record('redirect hops reuse request ID, each hop completes',redirectCounts=[0,1,2],statuses=[302,307,200])
  aborted=events('/slow?abort');assert methods(aborted)==[before,started,error];assert aborted[-1]['event']['params']['errorText']=='NS_BINDING_ABORTED'
  record('abort uses fetchError with no responseCompleted')
  root=events('/main')[0]['event']['params']['context']
  urls=[]
  for suffix in ['/frame?same','/frame?cross']:
   child=events(suffix)[0];childid=child['event']['params']['context'];assert childid!=root
   created=next(o for o in rows if o['kind']=='event' and o['event']['method']=='browsingContext.contextCreated' and o['event']['params']['context']==childid)
   assert created['event']['params']['parent']==root and created['t']<child['t']
   fetch=[e for e in events('/slow?frame') if e['event']['params']['context']==childid]
   assert methods(fetch)==[before,started,completed]
   urls.append(child['event']['params']['request']['url'])
  assert urlparse(urls[0]).netloc!=urlparse(urls[1]).netloc
  record('same and cross-origin iframe requests belong to child IDs',origins=[urlparse(u).netloc for u in urls])
 late=events('/slow?subscribe');assert methods(late)==[before]
 body=next(o for o in rows if o['kind']=='body_end' and o['path']=='/slow?subscribe')
 assert body['t']<late[0]['t']
 subscribe=next(o for o in rows if o['kind']=='command' and o['method']=='session.subscribe' and '/slow?subscribe' in ''.join(str(x) for x in rows[:rows.index(o)]) and o['t']<late[0]['t'])
 ack=next(o for o in rows if o['kind']=='reply' and o['reply']['id']==subscribe['id']);assert ack['t']<body['t']
 end=next((o['t'] for o in rows if o['kind']=='scenario' and o['name']=='fresh-request-after-late-subscription'),next(o['t'] for o in rows if o['kind']=='result'))
 record('late subscription emits delayed beforeRequestSent only',subscription_ack_t=ack['t'],server_body_end_t=body['t'],late_start_t=late[0]['t'],no_terminal_observation_seconds=round(end-late[0]['t'],6))
 if name!='run1':
  assert methods(events('/slow?after-subscribe'))==[before,started,completed]
  record('requests begun after subscription have normal terminal events')
  broken=events('/broken');assert methods(broken)==[before,started,error];assert broken[-1]['event']['params']['errorText']=='NS_ERROR_NET_PARTIAL_TRANSFER'
  record('short body uses fetchError NS_ERROR_NET_PARTIAL_TRANSFER')
  child=events('/frame?remove')[0]['event']['params']['context'];fetch=[o for o in events('/slow?frame') if o['event']['params']['context']==child]
  assert methods(fetch)==[before,started,error]
  destroyed=next(o for o in rows if o['kind']=='event' and o['event']['method']=='browsingContext.contextDestroyed' and o['event']['params']['context']==child)
  assert fetch[-1]['t']<destroyed['t'];record('removing active iframe aborts request before contextDestroyed')
  old=events('/slow?old-navigation');assert methods(old)==[before,started,error]
  new=events('/new-document');assert methods(new)==[before,started,completed]
  assert new[0]['t']<old[-1]['t'];record('new navigation starts before old document fetch aborts')
 assert next(o['success'] for o in rows if o['kind']=='result')
 metadata=next(o for o in rows if o['kind']=='metadata')
 reports.append({'run':name,'firefox_version':cap['capabilities']['browserVersion'],'metadata':metadata,'checks':checks})
(OUT/'summary.json').write_text(json.dumps(reports,indent=2)+'\n')
for name,script in [('run2','probe.py'),('run3','fresh-subscription.py')]:shutil.copy(OUT/script,OUT/name/'probe.py')
snapshot=OUT/'inspected-source';snapshot.mkdir(exist_ok=True)
root=OUT.parent.parent
for name in ['crates/rustwright-core/src/network_idle.rs','crates/rustwright-core/src/page.rs','crates/rustwright-bidi/src/network.rs','crates/rustwright-bidi/src/session.rs','crates/rustwright-bidi/src/browser.rs','crates/rustwright/src/any.rs']:
 target=snapshot/name;target.parent.mkdir(parents=True,exist_ok=True);shutil.copy(root/name,target)
files=[p for p in OUT.rglob('*') if p.is_file() and not any(x.startswith('profile-') for x in p.parts) and p.name not in ['sha256.json']]
(OUT/'sha256.json').write_text(json.dumps({str(p.relative_to(OUT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(files)},indent=2)+'\n')
print(json.dumps([{'run':r['run'],'version':r['firefox_version'],'validated_observations':len(r['checks'])} for r in reports],indent=2))
