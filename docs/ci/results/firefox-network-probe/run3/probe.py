import asyncio,json,os,re,tempfile,time,hashlib,sys,signal
from pathlib import Path
from websockets.asyncio.client import connect
OUT=Path(os.environ.get("PROBE_OUTPUT",str(Path(__file__).resolve().parent)))
OUT.mkdir(parents=True,exist_ok=True)
rows=[]; start=time.monotonic()
def emit(kind,**kw):
    obj={'t':round(time.monotonic()-start,6),'kind':kind,**kw};rows.append(obj)
    with (OUT/'events.jsonl').open('a') as f:f.write(json.dumps(obj)+'\n')
class Protocol:
    def __init__(self,ws):self.ws=ws;self.pending={};self.n=0
    async def reader(self):
        async for raw in self.ws:
            o=json.loads(raw)
            if 'id' in o:
                emit('reply',reply=o)
                f=self.pending.pop(o['id'],None)
                if f and not f.done(): f.set_result(o)
            else:emit('event',event=o)
    async def cmd(self,m,p):
        self.n+=1; n=self.n;f=asyncio.get_running_loop().create_future();self.pending[n]=f
        emit('command',id=n,method=m,params=p)
        await self.ws.send(json.dumps({'id':n,'method':m,'params':p}))
        o=await asyncio.wait_for(f,10)
        if o.get('type')=='error':raise RuntimeError(o)
        return o.get('result',{})
async def fixture(r,w):
    try:
        q=await asyncio.wait_for(r.readuntil(b'\r\n\r\n'),3);path=q.split(b' ')[1].decode();emit('http',path=path)
        if path.startswith('/redirect1'):
            w.write(b'HTTP/1.1 302 Found\r\nLocation: /redirect2\r\nContent-Length: 0\r\nConnection: close\r\n\r\n')
        elif path.startswith('/redirect2'):
            w.write(b'HTTP/1.1 307 Temporary Redirect\r\nLocation: /slow?redirect\r\nContent-Length: 0\r\nConnection: close\r\n\r\n')
        elif path.startswith('/broken'):
            w.write(b'HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nbegin');await w.drain();await asyncio.sleep(.1)
        elif path.startswith('/slow'):
            w.write(b'HTTP/1.1 200 OK\r\nContent-Length: 8\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-store\r\nConnection: close\r\n\r\nbegin');await w.drain();emit('headers',path=path)
            await asyncio.sleep(1);w.write(b'end');emit('body_end',path=path)
        elif path.startswith('/frame'):
            b=b'<title>child</title><script>fetch("/slow?frame").then(r=>r.text()).then(t=>window.done=t)</script>'
            w.write(b'HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: '+str(len(b)).encode()+b'\r\nConnection: close\r\n\r\n'+b)
        else:
            b=b'<title>network-probe</title><body>probe</body>';w.write(b'HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: '+str(len(b)).encode()+b'\r\nConnection: close\r\n\r\n'+b)
        await w.drain()
    except (ConnectionError,asyncio.IncompleteReadError):pass
    finally:w.close();await w.wait_closed()
async def main():
    (OUT/'events.jsonl').write_text(''); servers=[];proc=None;stderr_task=None;tasks=set()
    def accept(r,w):
        t=asyncio.create_task(fixture(r,w));tasks.add(t);t.add_done_callback(tasks.discard)
    try:
        servers=[await asyncio.start_server(accept,'127.0.0.1',0),await asyncio.start_server(accept,'127.0.0.1',0)]
        base,other=[f'http://127.0.0.1:{s.sockets[0].getsockname()[1]}' for s in servers]
        with tempfile.TemporaryDirectory(prefix='profile-',dir=OUT) as profile:
            cmd=[os.environ['RUSTWRIGHT_FIREFOX'],'--headless','--no-remote','--profile',profile,'--remote-debugging-port=0','about:blank']
            proc=await asyncio.create_subprocess_exec(*cmd,stdout=asyncio.subprocess.DEVNULL,stderr=asyncio.subprocess.PIPE,start_new_session=True)
            endpoint=asyncio.get_running_loop().create_future()
            async def stderr():
                with (OUT/'firefox.stderr').open('wb') as f:
                    while line:=await proc.stderr.readline():
                        f.write(line);f.flush()
                        if b'WebDriver BiDi listening on' in line:
                            m=re.search(rb'ws://[^\s]+',line)
                            if m and not endpoint.done():endpoint.set_result(m.group().decode().rstrip('/')+'/session')
            stderr_task=asyncio.create_task(stderr());address=await asyncio.wait_for(endpoint,10)
            binary=Path(f'/proc/{proc.pid}/exe').resolve()
            emit('metadata',command=cmd,binary=str(binary),binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),python=sys.version)
            async with connect(address,proxy=None) as ws:
                p=Protocol(ws);reader=asyncio.create_task(p.reader())
                emit('capabilities',value=await p.cmd('session.new',{'capabilities':{}}))
                context=(await p.cmd('browsingContext.create',{'type':'tab'}))['context']
                await p.cmd('session.subscribe',{'events':['browsingContext.contextCreated','browsingContext.contextDestroyed']})
                await p.cmd('browsingContext.navigate',{'context':context,'url':base+'/main','wait':'complete'})
                async def evaluate(expr):return await p.cmd('script.evaluate',{'expression':expr,'target':{'context':context},'awaitPromise':False})
                emit('scenario',name='subscribe-during-active-request');await evaluate('fetch("/slow?subscribe").then(r=>r.text())');await asyncio.sleep(.1)
                await p.cmd('session.subscribe',{'events':['network.beforeRequestSent','network.responseStarted','network.responseCompleted','network.fetchError']});await asyncio.sleep(3.0)
                emit('scenario',name='fresh-request-after-late-subscription');await evaluate('fetch("/slow?after-subscribe").then(r=>r.text())');await asyncio.sleep(1.3)
                emit('scenario',name='early-body-close');await evaluate('fetch("/broken").then(r=>r.text()).catch(()=>{})');await asyncio.sleep(.7)
                emit('scenario',name='child-removal-in-flight');await evaluate('(()=>{let f=document.createElement("iframe");f.id="remove";f.src="/frame?remove";document.body.append(f)})()');await asyncio.sleep(.1);await evaluate('document.getElementById("remove").remove()');await asyncio.sleep(1.2)
                emit('scenario',name='navigation-replaces-pending-fetch');await evaluate('fetch("/slow?old-navigation").then(r=>r.text()).catch(()=>{})');await asyncio.sleep(.1);await p.cmd('browsingContext.navigate',{'context':context,'url':base+'/new-document','wait':'complete'});await asyncio.sleep(1.2)
                await p.cmd('browsingContext.close',{'context':context});reader.cancel();await asyncio.gather(reader,return_exceptions=True)
            emit('result',success=True)
    finally:
        for s in servers:s.close();await s.wait_closed()
        if proc and proc.returncode is None:
            os.killpg(proc.pid,signal.SIGTERM)
            try:await asyncio.wait_for(proc.wait(),5)
            except asyncio.TimeoutError:os.killpg(proc.pid,signal.SIGKILL);await proc.wait()
        if stderr_task:await asyncio.gather(stderr_task,return_exceptions=True)
        for t in list(tasks):t.cancel()
        await asyncio.gather(*tasks,return_exceptions=True)
        emit('cleanup',browser_returncode=proc.returncode if proc else None)
asyncio.run(main())
