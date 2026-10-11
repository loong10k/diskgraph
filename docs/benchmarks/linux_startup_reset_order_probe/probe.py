import socket, struct, json, collections,platform
results=[]
for packet in [(1,0,0),(2,99,13),None]:
 observations=collections.Counter()
 for _ in range(1000):
  parent,child=socket.socketpair(socket.AF_UNIX,socket.SOCK_SEQPACKET)
  parent.send(struct.pack('=III',3,0,0))
  if packet:child.send(struct.pack('=III',*packet))
  child.close()
  sequence=[]
  for index in range(2):
   try:
    data=parent.recv(12,socket.MSG_DONTWAIT|socket.MSG_TRUNC)
    sequence.append(('packet:'+str(struct.unpack('=III',data))) if data else 'EOF')
   except OSError as error:sequence.append('errno:'+str(error.errno))
  observations[str(sequence)]+=1
  parent.close()
 results.append({'queued_packet':packet,'observations':dict(observations)})
print(json.dumps({'platform':platform.uname()._asdict(),'scope':'raw Linux seqpacket receive ordering; no production handshake qualification','results':results},indent=2))
