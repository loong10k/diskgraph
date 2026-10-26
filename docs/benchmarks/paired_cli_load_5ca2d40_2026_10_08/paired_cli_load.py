import subprocess,pathlib,json,os,hashlib,time
out=pathlib.Path('/output')
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
binaries={name:{binary:sha(out/(name+'_bin')/binary) for binary in ['diskgraph','diskgraph-scan-worker']} for name in ['baseline','candidate']}
records=[]
for count in [20000,200000]:
    for round_number,order in enumerate([['baseline','candidate'],['candidate','baseline']],1):
        for name in order:
            bindir=out/(name+'_bin');image=bindir/'diskgraph-scan-worker'
            assert all(sha(bindir/b)==digest for b,digest in binaries[name].items())
            label=f'paired-{count}-round{round_number}-{name}';env=dict(os.environ,DISKGRAPH_SCAN_WORKER_PATH=str(image),DISKGRAPH_SCAN_WORKER_SHA256=binaries[name]['diskgraph-scan-worker'],DISKGRAPH_SCAN_WORKER_BYTES=str(image.stat().st_size))
            command=['python3','scripts/accept-readonly-load.py','--bin-dir',str(bindir),'--files',str(count),'--queries','32','--output',str(out/(label+'.json'))]
            with (out/(label+'.log')).open('wb') as log:result=subprocess.run(command,env=env,stdout=log,stderr=subprocess.STDOUT)
            assert result.returncode==0,(label,result.returncode)
            assert all(sha(bindir/b)==digest for b,digest in binaries[name].items())
            records.append({'label':label,'report':json.loads((out/(label+'.json')).read_text())})
            (out/'paired-cli-receipt.json').write_text(json.dumps({'baseline':'1cc15c244e3dbb7f04cda25d392cc07dd4fce03e','candidate':'5ca2d40f23c210f890d7714620e2c115f5229807','binary_sha256':binaries,'records':records,'limits':'AB/BA ordering, independent generated identical-shape fixtures; no CPU cache flush; no accepted SLO; no RSS comparison; Linux ARM64 only','production_acceptance':False},indent=2)+'\n')
