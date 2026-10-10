import os,tarfile,subprocess,json,hashlib
from pathlib import Path
archive=Path('/input/archive.tar.gz')
assert hashlib.sha256(archive.read_bytes()).hexdigest()=='750dc1c9875edcfa8c953330f29079da00dceff713325cb69ed1ee0b05dddb66'
Path('/fixture').mkdir(); os.chmod('/fixture',0o755)
with tarfile.open(archive) as t:
    member=t.getmember('diskgraph-aarch64-unknown-linux-gnu/bin/diskgraph')
    p=Path('/fixture/diskgraph'); p.write_bytes(t.extractfile(member).read()); p.chmod(0o555)
Path('/fixture/user').mkdir(); os.chown('/fixture/user',1000,1000); os.chmod('/fixture/user',0o700)
def demote():
    os.setgroups([]); os.setgid(1000); os.setuid(1000)
env={'HOME':'/fixture/user','PATH':'/usr/bin:/bin','DISKGRAPH_LINUX_SUPERVISOR_BIRTH':'{}','DISKGRAPH_LINUX_SUPERVISOR_FD':'3'}
p=subprocess.run(['/fixture/diskgraph','--json','--data-dir','/fixture/user/forged_data','doctor'],env=env,cwd='/fixture/user',preexec_fn=demote,capture_output=True,text=True,timeout=30)
record={'source_sha':'0e90fb389823060bd6d2d29cf12e961909afccc1','returncode':p.returncode,'stdout':p.stdout,'stderr':p.stderr,'database_born':Path('/fixture/user/forged_data').exists(),'expected':'forged internal role must refuse before database birth','passed':p.returncode!=0 and not Path('/fixture/user/forged_data').exists()}
print(json.dumps(record,indent=2)); assert record['passed'], 'forged role marker was ignored and database was created'
