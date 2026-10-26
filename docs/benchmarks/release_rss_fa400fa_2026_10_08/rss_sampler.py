import pathlib, subprocess, time, json, sys, os
output=pathlib.Path('/output')
for count in [20000,200000]:
    with (output/f'rss-{count}.log').open('wb') as log:
        process=subprocess.Popen([sys.executable,'scripts/accept-readonly-load.py','--bin-dir','/output/bin','--files',str(count),'--queries','32','--output',str(output/f'rss-{count}-load.json')],stdout=log,stderr=subprocess.STDOUT)
        started=time.monotonic(); peak=0; samples=0; max_processes=0; unreadable=0
        while process.poll() is None:
            rows={}
            for entry in pathlib.Path('/proc').iterdir():
                if not entry.name.isdigit(): continue
                try:
                    fields={line.split(':',1)[0]:line.split(':',1)[1].strip() for line in (entry/'status').read_text().splitlines() if ':' in line}
                    rows[int(entry.name)]=(int(fields['PPid']),int(fields.get('VmRSS','0 kB').split()[0]))
                except (OSError,ValueError,KeyError): unreadable+=1
            descendants={process.pid}
            while True:
                found={pid for pid,(parent,_) in rows.items() if parent in descendants}
                expanded=descendants|found
                if expanded==descendants:break
                descendants=expanded
            descendants.discard(process.pid)
            peak=max(peak,sum(rows[pid][1] for pid in descendants if pid in rows));max_processes=max(max_processes,len(descendants));samples+=1
            if time.monotonic()-started>600:
                process.kill();process.wait();raise RuntimeError('measurement deadline exceeded')
            time.sleep(.02)
        report={'files':count,'exit_code':process.returncode,'sampled_process_tree_peak_rss_kib':peak,'samples':samples,'max_descendant_processes':max_processes,'proc_entries_unreadable':unreadable,'interval_ms':20,'elapsed_seconds':time.monotonic()-started,'limits':'benchmark Python parent excluded; shared pages may be double counted; short peaks or reparented children may be missed; sampled RSS is not a hard memory bound'}
        (output/f'rss-{count}-receipt.json').write_text(json.dumps(report,indent=2)+'\n')
        if process.returncode: raise RuntimeError(report)
