import json,pathlib,tarfile,hashlib,sys
ROOT=pathlib.Path(__file__).resolve().parent
SRC={}
with tarfile.open(ROOT.parent/'source-fadb346/reproducible-source.tar.gz') as archive:
 for m in archive.getmembers():
  if m.isfile():SRC[m.name]=archive.extractfile(m).read()
report=json.loads((ROOT/'artifacts/release-verification/report.json').read_text());assert report['status']=='passed'
merge='87f6180e4922ca860e10f91701e6032e6cb85390';result=[]
for package in report['rustwright_packages']:
 files=list((ROOT/'artifacts/release-verification').rglob(pathlib.Path(package['archive']).name));assert len(files)==1
 path=files[0];assert hashlib.sha256(path.read_bytes()).hexdigest()==package['sha256'];assert all(package['checks'].values());matched=[]
 with tarfile.open(path) as archive:
  members={m.name.split('/',1)[1]:m for m in archive.getmembers() if m.isfile()};vcs=json.loads(archive.extractfile(members['.cargo_vcs_info.json']).read());assert vcs['git']['sha1']==merge and not vcs['git'].get('dirty',False)
  for relative,member in members.items():
   if relative in ['.cargo_vcs_info.json','Cargo.lock','Cargo.toml']:continue
   original='Cargo.toml' if relative=='Cargo.toml.orig' else relative
   source=f"crates/{package['name']}/{original}"
   if source in SRC:
    assert archive.extractfile(member).read()==SRC[source],source;matched.append(source)
 result.append({'name':package['name'],'archive':str(path.relative_to(ROOT)),'sha256':package['sha256'],'git_vcs_info':vcs,'all_audit_checks_passed':True,'source_files_independently_matched':matched})
inputs={path:content for path,content in SRC.items() if path.startswith('crates/') or path in ['Cargo.toml','Cargo.lock','docs/PACKAGE_README.md','LICENSE-MIT','LICENSE-APACHE']}
digest=hashlib.sha256(''.join(path+':'+hashlib.sha256(inputs[path]).hexdigest()+'\n' for path in sorted(inputs,key=pathlib.Path)).encode()).hexdigest();assert digest==report['package_input_sha256']
value={'archives':result,'archive_count':len(result),'source_files_independently_matched':sum(len(x['source_files_independently_matched']) for x in result),'recomputed_package_input_sha256':digest,'package_input_files':len(inputs),'actual_archive_vcs_commit':merge,'third_party_verified_archives':report['third_party_verified_archives'],'readme_snippets':report['readme_snippets'],'native_backends':report['native_backends']}
(ROOT/'package-archive-verification.json').write_text(json.dumps(value,indent=2)+'\n');print({k:v for k,v in value.items() if k not in ['archives','readme_snippets','native_backends']})
