use diskgraph_core::{DiskGraph,DiskNode,DiskSnapshot,EvidenceEdge,ResourceLocator,EvidenceRelation,NodeKind,ScanCoverage,ScanSettings};
use diskgraph_store::{ControlStore,SqliteSnapshotStore,StoreError};
use std::time::{Instant,Duration};
fn main()->Result<(),Box<dyn std::error::Error>> {
 let dir=tempfile::tempdir()?; let graph=dir.path().join("graph.sqlite"); let ctl=dir.path().join("control.sqlite");
 let mut writer=SqliteSnapshotStore::open(&graph)?; writer.publish_revision("fixture-job",&fixture_graph("snapshot",1),"bound",1)?; let control=ControlStore::open(&ctl)?;
 let c=rusqlite::Connection::open(&ctl)?; c.execute("INSERT INTO server VALUES(1,'fixture',1)",[])?;drop(c);
 let mode=std::env::args().nth(1).unwrap_or_else(||"missing".into());
 let c=rusqlite::Connection::open(&graph)?;c.execute_batch("PRAGMA foreign_keys=ON; INSERT INTO revision_ownership VALUES('bound','fixture','scope');")?;
 if mode=="quarantined" {c.execute("INSERT INTO revision_access_denials VALUES('bound','fixture','scope','root_identity_unconfirmed')",[])?;}drop(c);
 let revision=if mode=="missing" {"absent"} else {"bound"};
 let expected=mode=="positive";
 let mut phases=[Vec::new(),Vec::new(),Vec::new(),Vec::new()];
 for i in 0..1100 {let deadline=Instant::now()+Duration::from_millis(50);let start=Instant::now();
 let server=control.with_read_deadline::<_,StoreError>(deadline,|c| c.existing_server_id())?;let a=Instant::now();
 let reader=SqliteSnapshotStore::open_reader_until(&graph,deadline,None)?;let b=Instant::now();
 assert_eq!(reader.revision_ownership_matches(revision,server.as_str(),"scope")?,expected);let end=Instant::now();drop(reader);let closed=Instant::now();
 if i>=100 {for (v,t) in phases.iter_mut().zip([a-start,b-a,end-b,closed-end]) {v.push(t.as_nanos() as u64);}}
 }
 let names=["control_existing_server_with_deadline","fresh_reader_open_configure","ownership_filtered_query","reader_drop"];
 for (name,mut v) in names.into_iter().zip(phases) {v.sort_unstable();println!("{}",serde_json::json!({"phase":name,"mode":mode,"samples":v.len(),"p50_ns":v[v.len()/2],"p95_ns":v[v.len()*95/100]}));}
 drop(writer);Ok(())
}
fn fixture_graph(id: &str, bytes: u64) -> DiskGraph {
    let root = ResourceLocator::NativePath("/tmp/diskgraph-test".into());
    DiskGraph {
        snapshot: DiskSnapshot {
            id: id.into(),
            root: root.clone(),
            volume_id: Some("test-volume".into()),
            captured_at_unix_ms: bytes,
            settings: ScanSettings {
                apparent_size: false,
                follow_links: false,
                include_hidden: true,
                one_filesystem: true,
                max_depth: None,
                dedup_hardlinks: true,
            },
            coverage: ScanCoverage {
                complete: true,
                unreadable_nodes: 0,
                depth_limited: false,
            },
        },
        nodes: vec![
            DiskNode {
                id: 1,
                parent_id: None,
                locator: root,
                name: "diskgraph-test".into(),
                kind: NodeKind::Directory,
                subtree_bytes: bytes,
                size_known: true,
                direct_bytes: 0,
                files: 1,
                directories: 1,
                modified_unix_seconds: None,
                file_identity: None,
                category_hint: None,
                reclaim_hint: None,
                read_error: false,
            },
            DiskNode {
                id: 2,
                parent_id: Some(1),
                locator: ResourceLocator::NativePath("/tmp/diskgraph-test/cache".into()),
                name: "cache".into(),
                kind: NodeKind::Directory,
                subtree_bytes: bytes,
                size_known: true,
                direct_bytes: bytes,
                files: 1,
                directories: 1,
                modified_unix_seconds: None,
                file_identity: None,
                category_hint: None,
                reclaim_hint: None,
                read_error: false,
            },
        ],
        evidence: vec![EvidenceEdge {
            node_id: 2,
            relation: EvidenceRelation::Rebuildable,
            subject: "test".into(),
            source: "unit-test".into(),
            observed_at_unix_ms: bytes,
            confidence: 80,
        }],
    }
}

