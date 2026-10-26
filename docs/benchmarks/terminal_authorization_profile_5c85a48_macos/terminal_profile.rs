use diskgraph_store::{ControlStore,SqliteSnapshotStore,StoreError};
use std::time::{Instant,Duration};
fn main()->Result<(),Box<dyn std::error::Error>> {
 let dir=tempfile::tempdir()?; let graph=dir.path().join("graph.sqlite"); let ctl=dir.path().join("control.sqlite");
 let writer=SqliteSnapshotStore::open(&graph)?; let control=ControlStore::open(&ctl)?;
 let c=rusqlite::Connection::open(&ctl)?; c.execute("INSERT INTO server VALUES(1,'fixture',1)",[])?;drop(c);
 let mut phases=[Vec::new(),Vec::new(),Vec::new(),Vec::new()];
 for i in 0..1100 {let deadline=Instant::now()+Duration::from_millis(50);let start=Instant::now();
 let server=control.with_read_deadline::<_,StoreError>(deadline,|c| c.existing_server_id())?;let a=Instant::now();
 let reader=SqliteSnapshotStore::open_reader_until(&graph,deadline,None)?;let b=Instant::now();
 assert!(!reader.revision_ownership_matches("absent",server.as_str(),"scope")?);let end=Instant::now();drop(reader);let closed=Instant::now();
 if i>=100 {for (v,t) in phases.iter_mut().zip([a-start,b-a,end-b,closed-end]) {v.push(t.as_nanos() as u64);}}
 }
 let names=["control_existing_server_with_deadline","fresh_reader_open_configure","ownership_negative_query","reader_drop"];
 for (name,mut v) in names.into_iter().zip(phases) {v.sort_unstable();println!("{}",serde_json::json!({"phase":name,"samples":v.len(),"p50_ns":v[v.len()/2],"p95_ns":v[v.len()*95/100]}));}
 drop(writer);Ok(())
}
