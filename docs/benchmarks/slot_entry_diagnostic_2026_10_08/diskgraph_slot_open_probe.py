import os,sys,tempfile,multiprocessing as mp,json,time,platform
from pathlib import Path

def child(root,start,queue):
    directory=os.open(root,os.O_RDONLY|os.O_DIRECTORY|os.O_CLOEXEC|os.O_NOFOLLOW)
    start.wait()
    errors=[]
    attempts=10000
    flags=os.O_RDWR|os.O_CREAT|os.O_NOFOLLOW|os.O_CLOEXEC|os.O_NONBLOCK
    try:
        for i in range(attempts):
            try:
                fd=os.open('supervisor_'+str(i%4)+'.slot',flags,0o600,dir_fd=directory)
                os.close(fd)
            except OSError as error:
                errors.append({'iteration':i,'errno':error.errno,'parent_inode':os.fstat(directory).st_ino,'named_parent_inode':os.stat(root).st_ino})
                if len(errors)>=20: break
        queue.put({'pid':os.getpid(),'attempts':i+1,'errors':errors})
    finally: os.close(directory)

if __name__=='__main__':
    context=mp.get_context('spawn'); started=time.monotonic()
    with tempfile.TemporaryDirectory(prefix='diskgraph-slot-open-') as root:
        os.chmod(root,0o700); start=context.Event(); queue=context.Queue()
        children=[context.Process(target=child,args=(root,start,queue)) for _ in range(8)]
        try:
            for process in children: process.start()
            start.set()
            rows=[queue.get(timeout=30) for _ in children]
            for process in children: process.join(timeout=5)
            if any(process.is_alive() or process.exitcode!=0 for process in children): raise RuntimeError('child not normally reaped')
            report={'platform':platform.platform(),'machine':platform.machine(),'processes':8,'elapsed_seconds':time.monotonic()-started,'total_attempts':sum(row['attempts'] for row in rows),'errors':sum(len(row['errors']) for row in rows),'results':rows,'scope':'isolated openat flag reproduction only; no recovery slot protocol or managed-worker acceptance'}
            Path('/tmp/diskgraph_slot_open_probe.json').write_text(json.dumps(report,indent=2)+'\n')
            print(json.dumps({key:report[key] for key in ['platform','processes','elapsed_seconds','total_attempts','errors','scope']}))
        finally:
            for process in children:
                if process.pid is not None:
                    if process.is_alive(): process.kill()
                    process.join()
