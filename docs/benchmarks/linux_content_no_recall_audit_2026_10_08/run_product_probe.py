import os,stat,pathlib,subprocess,time,ctypes,json
os.mknod("/dev/fuse",stat.S_IFCHR|0o600,os.makedev(10,229))
root=pathlib.Path("/tmp/private-provider");root.mkdir();log=open("/fixture/current_provider.log","w+")
p=subprocess.Popen(["/tmp/fuse-provider",str(root)],stdout=log,stderr=log)
try:
 for _ in range(200):
  log.seek(0)
  if "PROVIDER_MOUNTED" in log.read():break
  if p.poll() is not None:raise RuntimeError("provider failed")
  time.sleep(.01)
 else:raise RuntimeError("mount readiness timeout")
 env=dict(os.environ,DG_FUSE_ROOT=str(root),DG_FUSE_LOG="/fixture/current_provider.log")
 with open("/fixture/current_product.log","w") as output:
  result=subprocess.run(["cargo","test","-p","diskgraph-engine","--test","linux_content_no_recall_probe","--locked","--","--nocapture","--test-threads=1"],env=env,stdout=output,stderr=subprocess.STDOUT,timeout=int(os.environ.get("DG_FUSE_TEST_TIMEOUT", "180")))
 print("PRODUCT_TEST_EXIT",result.returncode)
 text=pathlib.Path("/fixture/current_product.log").read_text();print(text[-4500:]);assert result.returncode==0
 log.seek(0);assert "FUSE_FETCH_DATA" not in log.read(),"product content recall"
 assert (root/"placeholder").read_bytes()==b"synthetic-cloud-content"
 log.seek(0);text=log.read();print(text);assert "FUSE_DATA_OPEN" in text and "FUSE_FETCH_DATA=1" in text
finally:
 libc=ctypes.CDLL(None,use_errno=True)
 if libc.umount2(bytes(root),0)!=0:raise OSError(ctypes.get_errno(),"actual unmount failed")
 p.wait(timeout=3);log.close();assert p.returncode==0,p.returncode
 print("ORIGINAL_PROVIDER_UNMOUNTED_AND_REAPED=1")
