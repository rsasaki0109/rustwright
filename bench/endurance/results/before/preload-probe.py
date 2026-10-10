import asyncio,json,os,socket,subprocess,tempfile,time
from pathlib import Path
from websockets.asyncio.client import connect

async def probe(port):
    for _ in range(100):
        try:
            ws=await connect(f'ws://127.0.0.1:{port}/session',proxy=None,open_timeout=1)
            break
        except (OSError,TimeoutError):
            await asyncio.sleep(.1)
    else: raise RuntimeError('Firefox did not start')
    async with ws:
        counter=0
        async def command(method,params):
            nonlocal counter
            counter+=1
            await ws.send(json.dumps({'id':counter,'method':method,'params':params}))
            while True:
                value=json.loads(await asyncio.wait_for(ws.recv(),5))
                if value.get('id')==counter:return value
        print(json.dumps(await command('session.new',{'capabilities':{}})))
        ctx=await command('browser.createUserContext',{})
        page=await command('browsingContext.create',{'type':'tab','userContext':ctx['result']['userContext']})
        script=await command('script.addPreloadScript',{'functionDeclaration':'function(){window.probe=true}', 'contexts':[page['result']['context']]})
        print(json.dumps({'script':script}))
        print(json.dumps({'close_context':await command('browser.removeUserContext',{'userContext':ctx['result']['userContext']})}))
        print(json.dumps({'remove_after_context_closed':await command('script.removePreloadScript',{'script':script['result']['script']})}))
        await command('session.end',{})

with tempfile.TemporaryDirectory(prefix='rustwright-preload-probe-') as profile:
    with socket.socket() as listener:
        listener.bind(('127.0.0.1',0));port=listener.getsockname()[1]
    with open('/tmp/rustwright-preload-probe.stderr','w') as log:
        process=subprocess.Popen([os.environ['RUSTWRIGHT_FIREFOX'],'--headless','--no-remote','--profile',profile,f'--remote-debugging-port={port}','about:blank'],stdout=subprocess.DEVNULL,stderr=log)
        try:asyncio.run(probe(port))
        finally:
            process.terminate()
            try:process.wait(timeout=5)
            except subprocess.TimeoutExpired:process.kill();process.wait()
