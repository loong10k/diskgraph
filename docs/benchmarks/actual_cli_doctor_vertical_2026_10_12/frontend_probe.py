import socket,os,subprocess,tempfile,json,struct,time,pathlib,fcntl
rows=[]
def lowered():
 os.setgroups([]);os.setgid(23112);os.setuid(23112)
def run(binary,mode,expected):
 with tempfile.TemporaryDirectory() as root:
  os.chmod(root,0o755); path=root+'/front.sock'
  server=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);server.setsockopt(socket.SOL_SOCKET,socket.SO_PASSCRED,1);server.bind(path);os.chmod(path,0o666);server.listen(1)
  child=subprocess.Popen([binary,'frontend',path,'doctor'],preexec_fn=lowered,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
  pipe_capacity=fcntl.fcntl(child.stdout.fileno(),1032)
  if mode=="nonreading":pipe_capacity=fcntl.fcntl(child.stdout.fileno(),1031,4096)
  started=time.monotonic();connection,_=server.accept();connection.settimeout(10)
  peer=struct.unpack('3i',connection.getsockopt(socket.SOL_SOCKET,socket.SO_PEERCRED,12));assert peer==(child.pid,23112,23112)
  request=b''
  while True:
   part=connection.recv(64)
   if not part:break
   request+=part;assert len(request)<=64
  assert request==b'DG_FRONT_INPUT\n'
  payload=(json.dumps({'ok':True,'data':{'text':'磁盘'*2048}},ensure_ascii=False)+'\n').encode()
  if mode=='oversized':payload=b'x'*131073
  if mode=='nonreading':payload=b'x'*(pipe_capacity+4096)
  try:
   if mode=='normal':
    for start in range(0,len(payload),4091):connection.sendall(payload[start:start+4091])
   else:connection.sendall(payload)
   connection.shutdown(socket.SHUT_WR)
  except (BrokenPipeError,ConnectionResetError):pass
  if mode=='nonreading':
   child.wait(timeout=35)
   assert time.monotonic()-started<33
  stdout,stderr=child.communicate(timeout=35)
  assert child.returncode==expected,(binary,mode,child.returncode,stderr)
  if mode=='normal' and expected==0:assert stdout==payload
  if mode=='oversized':assert len(stdout)<=131072
  rows.append({'binary':binary,'mode':mode,'actual_uid':peer[1],'actual_pid_matches_original_child':True,'exit':child.returncode,'stdout_pipe_capacity':pipe_capacity,'stdout_bytes':len(stdout),'seconds':time.monotonic()-started,'actual_original_wait_consumed':True,'payload_exact':stdout==payload})
  connection.close();server.close()
run('/old-role','normal',8)
run('/new-role','normal',0)
run('/new-role','oversized',8)
run('/new-role','nonreading',8)
pathlib.Path('/out/result.json').write_text(json.dumps({'fixture_only':True,'rust_actual_doctor_vertical_executed':False,'cases':rows},indent=2)+'\n')
print(json.dumps(rows))
