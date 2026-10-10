import hashlib, importlib.util, json, os, stat, subprocess, sys
from pathlib import Path
spec=importlib.util.spec_from_file_location('installation','/input/prepare_linux_supervisor_installation.py')
m=importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
assert sys.platform=='linux' and os.geteuid()==0
root=Path('/opt/diskgraph-supervisor-fixture')
expected='750dc1c9875edcfa8c953330f29079da00dceff713325cb69ed1ee0b05dddb66'
r=m.prepare('/input/package.tar.gz',expected,root,1000,1000,[1001])
checks={}
for name in m.NAMES:
 p=root/'images'/name
 info=p.stat(); expected_image=r['role_images'][name]
 assert info.st_uid==0 and stat.S_IMODE(info.st_mode)==0o555 and info.st_nlink==1
 assert hashlib.sha256(p.read_bytes()).hexdigest()==expected_image['sha256']
checks['actual_three_images_root_owned_digest_match']=True
for folder in ['images','state']:
 info=(root/folder).stat(); assert info.st_uid==0 and stat.S_IMODE(info.st_mode)==0o755
checks['protected_directory_identity']=True
bootstrap=root/'state/bootstrap.slot'; slot=root/'state/uid_1000.slot'
assert bootstrap.read_bytes()==slot.read_bytes()==b'DGSL01C\n'
assert bootstrap.stat().st_uid==0 and slot.stat().st_uid==1000
assert stat.S_IMODE(bootstrap.stat().st_mode)==stat.S_IMODE(slot.stat().st_mode)==0o600
checks['new_clean_slots_have_distinct_real_owners']=True
child = r"""import os,sys
from pathlib import Path
uid=int(sys.argv[1]); root=Path('/opt/diskgraph-supervisor-fixture')
os.setgroups([]); os.setgid(uid); os.setuid(uid)
assert os.getuid()==os.geteuid()==uid
if uid==1000:
 with (root/'state/uid_1000.slot').open('r+b') as f: assert f.read()==b'DGSL01C\n'
else:
 try: (root/'state/uid_1000.slot').open('r+b')
 except PermissionError: pass
 else: raise AssertionError('frontend could alter service slot')
for p in [root/'state/bootstrap.slot',root/'images/diskgraph']:
 try: p.open('r+b')
 except PermissionError: pass
 else: raise AssertionError('nonroot could alter protected deployment')
try: (root/'state/forged.slot').write_bytes(b'DGSL01C\n')
except PermissionError: pass
else: raise AssertionError('nonroot could replace namespace')
"""
for uid in [1000,1001]:
 p=subprocess.run([sys.executable,'-c',child,str(uid)],capture_output=True,text=True,timeout=10)
 assert p.returncode==0,(uid,p.stdout,p.stderr)
checks['actual_service_uid_can_open_only_its_slot']=True
checks['actual_frontend_uid_cannot_modify_service_or_namespace']=True
slot.write_bytes(b'DGSL01A\n')
original=(slot.read_bytes(),(root/'supervisor-deployment.json').read_bytes())
try: m.prepare('/input/package.tar.gz',expected,root,1000,1000,[1001])
except FileExistsError: pass
else: raise AssertionError('existing installation overwritten')
assert original==(slot.read_bytes(),(root/'supervisor-deployment.json').read_bytes())
checks['existing_active_record_and_config_preserved']=True
bad=Path('/opt/diskgraph-wrong-archive')
try: m.prepare('/input/package.tar.gz','0'*64,bad,1000,1000,[1001])
except ValueError: pass
else: raise AssertionError('wrong independent archive digest accepted')
assert not (bad/'supervisor-deployment.json').exists() and not (bad/'state').exists()
checks['wrong_digest_never_publishes_deployment']=True
link=Path('/opt/package-link'); link.symlink_to('/input/package.tar.gz')
try: m.prepare(link,expected,Path('/opt/diskgraph-link-refused'),1000,1000,[1001])
except OSError: pass
else: raise AssertionError('archive source link accepted')
assert not Path('/opt/diskgraph-link-refused').exists()
checks['source_link_refused_before_directory_birth']=True
assert r['services_started'] is False and r['broker_authenticated'] is False and r['supervisor_lifecycle_qualified'] is False
result={'checks':checks,'passed':len(checks),'total':len(checks),'actual_preparation':r,
 'installer_sha256':hashlib.sha256(Path('/input/prepare_linux_supervisor_installation.py').read_bytes()).hexdigest(),
 'qualification_sha256':hashlib.sha256(Path('/input/qualify.py').read_bytes()).hexdigest(),
 'limits':['Linux ARM64 root fixture only; broker and supervisor product entry are not implemented here.',
 'No service registration or service start; no desktop installation.',
 'Kernel UID tests qualify deployment permissions, not runtime peer authentication or pending I/O retirement.']}
Path('/output/receipt.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps({'passed':len(checks),'total':len(checks),'services_started':False}))
