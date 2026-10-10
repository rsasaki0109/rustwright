import os,json,urllib.request,urllib.error,urllib.parse,pathlib,sys,hashlib,zipfile
ROOT=pathlib.Path(__file__).resolve().parent
SECRET=os.environ.get('GH_TOKEN') or os.environ.get('GITHUB_TOKEN'); assert SECRET
BASE='https://api.github.com/repos/rsasaki0109/rustwright/'
class NoRedirect(urllib.request.HTTPRedirectHandler):
 def redirect_request(self,req,fp,code,msg,headers,newurl):return None
OPENER=urllib.request.build_opener(NoRedirect)
def fetch(path,destination):
 request=urllib.request.Request(BASE+path,headers={'Authorization':'Bearer '+SECRET,'Accept':'application/vnd.github+json','X-GitHub-Api-Version':'2022-11-28','User-Agent':'rustwright-ci-evidence'})
 try:response=OPENER.open(request,timeout=30)
 except urllib.error.HTTPError as error:
  if error.code not in [301,302,303,307,308]:raise
  location=error.headers['Location'];assert urllib.parse.urlparse(location).scheme=='https'
  response=urllib.request.urlopen(urllib.request.Request(location,headers={'User-Agent':'rustwright-ci-evidence'}),timeout=60)
 destination=ROOT/destination;destination.parent.mkdir(parents=True,exist_ok=True)
 with response,destination.open('wb') as stream:
  while block:=response.read(131072):stream.write(block)
 return destination
if __name__=='__main__':
 path,name=sys.argv[1:3];f=fetch(path,name)
 if name.endswith('.json'):
  d=json.loads(f.read_bytes())
  if 'jobs' in d:print(json.dumps([{'id':j['id'],'name':j['name'],'status':j['status'],'conclusion':j['conclusion']} for j in d['jobs']]))
  elif 'artifacts' in d:print(json.dumps([{'id':a['id'],'name':a['name'],'digest':a.get('digest'),'expired':a['expired']} for a in d['artifacts']]))
  else:print(json.dumps({k:d[k] for k in ['id','head_sha','status','conclusion','merge_commit_sha'] if k in d}))
 else:print(json.dumps({'file':name,'bytes':f.stat().st_size,'sha256':hashlib.sha256(f.read_bytes()).hexdigest()}))
