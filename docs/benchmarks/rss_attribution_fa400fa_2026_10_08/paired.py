import pathlib,subprocess,os,hashlib,json,sys
out=pathlib.Path('/output/rss_attribution_fa400fa')
bins={'baseline':pathlib.Path('/output/candidate_bin'),'candidate':pathlib.Path('/output/rss_fa400fa/bin')}
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
hashes={name:{p.name:sha(p) for p in folder.iterdir()} for name,folder in bins.items()}
assert hashes['baseline']['diskgraph']=='a8c8c469bb33916e2fbb8f9c5869def1adc8ba1a0a641aec4f57cca6b25493d1'
records=[]
for count in [200000]:
 for round_no,order in enumerate([['baseline','candidate']],1):
  for name in order:
   label=f'{count}-r{round_no}-{name}';folder=out/label;folder.mkdir();worker=bins[name]/'diskgraph-scan-worker'
   env=dict(os.environ,DISKGRAPH_SCAN_WORKER_PATH=str(worker),DISKGRAPH_SCAN_WORKER_SHA256=sha(worker),DISKGRAPH_SCAN_WORKER_BYTES=str(worker.stat().st_size))
   result=subprocess.run([sys.executable,str(out/'sampler.py'),str(count),str(folder),str(bins[name])],env=env,timeout=650)
   assert result.returncode==0,(label,result.returncode)
   assert all(sha(bins[name]/b)==h for b,h in hashes[name].items())
   records.append({'label':label,'load':json.loads((folder/f'rss-{count}-load.json').read_text()),'rss':json.loads((folder/f'rss-{count}-receipt.json').read_text())})
   (out/'receipt.json').write_text(json.dumps({'baseline':'5ca2d40f23c210f890d7714620e2c115f5229807','candidate':'fa400fa5e6119a85b9fad74ccfa8e18bdd56accb','binary_sha256':hashes,'records':records,'limits':'Linux ARM64 Docker AB/BA, sampled RSS perturbs workload, no accepted SLO, not hard peak, not other-platform qualification','production_acceptance':False},indent=2)+'\n')
