import subprocess,pathlib,json,os,hashlib
out=pathlib.Path('/output/release_b03de97')
bins={'baseline':pathlib.Path('/output/candidate_bin'),'candidate':out/'bin'}
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
binaries={n:{b:sha(p/b) for b in ['diskgraph','diskgraph-scan-worker']} for n,p in bins.items()}
assert binaries['baseline']['diskgraph']=='a8c8c469bb33916e2fbb8f9c5869def1adc8ba1a0a641aec4f57cca6b25493d1'
assert binaries['candidate']['diskgraph']=='8e7c736a99608f5f3008da65418c1511c968a8bc6fff36b8e1c06cd7f5c6c9fe'
records=[]
for count in [20000,200000]:
 for round_number,order in enumerate([['baseline','candidate'],['candidate','baseline']],1):
  for name in order:
   image=bins[name]/'diskgraph-scan-worker'
   env=dict(os.environ,DISKGRAPH_SCAN_WORKER_PATH=str(image),DISKGRAPH_SCAN_WORKER_SHA256=sha(image),DISKGRAPH_SCAN_WORKER_BYTES=str(image.stat().st_size))
   label=f'{count}-r{round_number}-{name}'
   with (out/(label+'.log')).open('wb') as log:
    result=subprocess.run(['python3','scripts/accept-readonly-load.py','--bin-dir',str(bins[name]),'--files',str(count),'--queries','32','--output',str(out/(label+'.json'))],env=env,stdout=log,stderr=subprocess.STDOUT,timeout=600)
   assert result.returncode==0,(label,result.returncode)
   assert all(sha(bins[name]/b)==v for b,v in binaries[name].items())
   records.append({'label':label,'report':json.loads((out/(label+'.json')).read_text())})
   (out/'receipt.json').write_text(json.dumps({'baseline':'5ca2d40f23c210f890d7714620e2c115f5229807','candidate':'b03de97b32e767c23b04e8308b876cd9eaee4a88','binary_sha256':binaries,'records':records,'platform':'Linux ARM64 Docker Rust1.97.1','limits':'AB/BA; fresh identical-shape fixtures; no accepted SLO; no RSS comparison; no native other OS; same worker binary','production_acceptance':False},indent=2)+'\n')
