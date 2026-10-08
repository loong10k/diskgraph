import os,fcntl,tempfile,concurrent.futures,time,json,stat

def probe(path):
 d=os.open(path,os.O_RDONLY|os.O_DIRECTORY|os.O_CLOEXEC)
 failures=[]; acquired=0;busy=0
 try:
  for iteration in range(1000):
   for index in range(4):
    name=f'supervisor_{index}.slot'
    try:f=os.open(name,os.O_RDWR|os.O_CREAT|os.O_NOFOLLOW|os.O_CLOEXEC|os.O_NONBLOCK,0o600,dir_fd=d)
    except OSError as e:
     failures.append({'stage':'open','errno':e.errno,'iteration':iteration,'slot':index});break
    try:
     m=os.fstat(f)
     if not stat.S_ISREG(m.st_mode) or m.st_nlink!=1 or stat.S_IMODE(m.st_mode)!=0o600:
      failures.append({'stage':'metadata'});break
     os.fsync(d)
     try:fcntl.flock(f,fcntl.LOCK_EX|fcntl.LOCK_NB)
     except BlockingIOError:busy+=1;continue
     for record in [b'DGSL01R\n',b'DGSL01C\n']:
      os.lseek(f,0,os.SEEK_SET);os.write(f,record);os.fsync(f)
      os.lseek(f,0,os.SEEK_SET)
      if os.read(f,9)!=record:failures.append({'stage':'record'})
     fcntl.flock(f,fcntl.LOCK_UN);acquired+=1;break
    finally:os.close(f)
   if failures:break
 finally:os.close(d)
 return {'acquired':acquired,'busy':busy,'failures':failures}

if __name__=='__main__':
 start=time.monotonic()
 with tempfile.TemporaryDirectory(prefix='diskgraph-slot-original-probe-') as p:
  os.chmod(p,0o700)
  with concurrent.futures.ProcessPoolExecutor(max_workers=4) as pool:results=list(pool.map(probe,[p]*4))
 print(json.dumps({'platform':os.uname().sysname,'processes':4,'attempts_per_process':1000,'elapsed_seconds':time.monotonic()-start,'results':results,'production_acceptance':False,'scope':'isolated syscall/lock/write/fsync probe; does not execute Engine startup'},indent=2))
