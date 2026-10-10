import importlib.util,pathlib,tempfile,time,json,concurrent.futures,statistics,math,sqlite3
spec=importlib.util.spec_from_file_location('load','scripts/accept-readonly-load.py');load=importlib.util.module_from_spec(spec);spec.loader.exec_module(load)
cli=pathlib.Path('/output/bin/diskgraph');out=pathlib.Path('/output/deep_current')
with tempfile.TemporaryDirectory(prefix='diskgraph-deep-cli-') as tmp:
    base=pathlib.Path(tmp);root=base/'root';root.mkdir();current=root
    for index in range(300):
        current=current/'d';current.mkdir();(current/'f.bin').write_bytes(b'x'*32)
    data=base/'data';scope=load.invoke(cli,data,'scope','add','--root',root)['data']['scope_id']
    started=time.perf_counter();indexed=load.invoke(cli,data,'index','--scope',scope,'--wait');scan=time.perf_counter()-started;revision=indexed['data']['revision_id']
    assert indexed['data']['state']=='completed' and revision
    with sqlite3.connect((data/'diskgraph.sqlite').as_uri()+'?mode=ro',uri=True) as connection:
        snapshot=connection.execute('SELECT snapshot_id FROM graph_revisions WHERE revision_id=?',(revision,)).fetchone()[0]
        indexed_count=connection.execute('SELECT COUNT(*) FROM nodes WHERE snapshot_id=?',(snapshot,)).fetchone()[0]
        maximum_depth=connection.execute('WITH RECURSIVE walk(id,depth) AS (SELECT id,0 FROM nodes WHERE snapshot_id=? AND parent_id IS NULL UNION ALL SELECT n.id,w.depth+1 FROM nodes n JOIN walk w ON n.parent_id=w.id WHERE n.snapshot_id=?) SELECT MAX(depth) FROM walk',(snapshot,snapshot)).fetchone()[0]
    assert indexed_count==601 and maximum_depth==301,(indexed_count,maximum_depth)
    def query(index):
        args=('top','--scope',scope) if index%2 else ('children','--scope',scope,'--limit','25')
        start=time.perf_counter();answer=load.invoke(cli,data,*args);return time.perf_counter()-start,answer
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:results=list(pool.map(query,range(32)))
    assert all(a['ok'] and a['data']['revision_id']==revision and a['data']['items'] for _,a in results)
    assert all(len(a['data']['items']) <= (20 if index%2 else 25) for index,(_,a) in enumerate(results))
    latencies=sorted(t*1000 for t,_ in results)
    report={'depth':300,'files':300,'directories_excluding_root':300,'scan_seconds':scan,'queries':32,'clients':4,'p50_ms':statistics.median(latencies),'p95_ms':latencies[math.ceil(.95*len(latencies))-1],'database_and_wal_bytes':sum(p.stat().st_size for p in data.glob('diskgraph*.sqlite*') if p.is_file()),'checks':{'completed_revision':True,'concurrent_revision_binding':True,'bounded_results':True,'exact_601_nodes_and_depth_301':True},'limits':'no RSS or baseline comparison; Linux ARM64 Docker only','production_acceptance':False}
    (out/'deep-cli-result.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
