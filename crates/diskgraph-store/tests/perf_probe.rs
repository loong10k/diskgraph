//! Real-data performance probes (run with --ignored): measures where the
//! time goes for a real 4.3M-node revision so optimization targets facts,
//! not guesses.

use std::time::Instant;

const DB: &str = "/tmp/diskgraph-demo/data/diskgraph.sqlite";

#[test]
#[ignore = "real-data probe over the operator's home revision"]
fn decompose_load_revision_cost() {
    let connection =
        rusqlite::Connection::open_with_flags(DB, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let snapshot_id: String = connection
        .query_row(
            "SELECT snapshot_id FROM graph_revisions WHERE revision_id = ?1",
            ["rev-c3bffcb9-e66b-430f-bc67-8bae3a24dd4c"],
            |row| row.get(0),
        )
        .unwrap();

    // Stage 1: raw SQLite read of every node_json row.
    let started = Instant::now();
    let mut statement = connection
        .prepare("SELECT node_json FROM nodes WHERE snapshot_id = ?1 ORDER BY id")
        .unwrap();
    let rows: Vec<String> = statement
        .query_map([&snapshot_id], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let read_seconds = started.elapsed().as_secs_f64();

    // Stage 2: deserialize every row into the full node model.
    let started = Instant::now();
    let mut nodes = Vec::with_capacity(rows.len());
    for json in &rows {
        nodes.push(serde_json::from_str::<diskgraph_core::DiskNode>(json).unwrap());
    }
    let parse_seconds = started.elapsed().as_secs_f64();

    // Stage 3: build the parent->children map (the tree renderer's shape).
    let started = Instant::now();
    let mut children_of: std::collections::HashMap<u64, Vec<&diskgraph_core::DiskNode>> =
        std::collections::HashMap::with_capacity(nodes.len());
    for node in &nodes {
        if let Some(parent) = node.parent_id {
            children_of.entry(parent).or_default().push(node);
        }
    }
    let map_seconds = started.elapsed().as_secs_f64();

    println!(
        "PROBE load_revision: read={read_seconds:.2}s parse={parse_seconds:.2}s map={map_seconds:.2}s nodes={}",
        nodes.len()
    );
}

#[test]
#[ignore = "real-data probe: batch insert throughput at different batch sizes"]
fn batch_insert_throughput_by_size() {
    let directory = tempfile::TempDir::with_prefix("dg-perf-batch-").unwrap();
    let database = directory.path().join("probe.sqlite");
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE nodes (
                 snapshot_id TEXT NOT NULL,
                 id INTEGER NOT NULL,
                 parent_id INTEGER,
                 locator_key TEXT NOT NULL,
                 name TEXT NOT NULL,
                 subtree_bytes INTEGER NOT NULL,
                 node_json TEXT NOT NULL,
                 PRIMARY KEY (snapshot_id, id)
             ) WITHOUT ROWID;",
        )
        .unwrap();
    let sample: String = "x".repeat(400);
    for batch_size in [512_usize, 2_048, 8_192, 32_768] {
        let total = 200_000_usize;
        let started = Instant::now();
        let mut id = 0_i64;
        let transaction = connection.unchecked_transaction().unwrap();
        {
            let mut statement = transaction
                .prepare("INSERT INTO nodes VALUES ('s', ?1, ?2, ?3, ?4, ?5, ?6)")
                .unwrap();
            for _ in 0..(total / batch_size) {
                for _ in 0..batch_size {
                    statement
                        .execute(rusqlite::params![id, 0, "k", "n", 4096_i64, sample])
                        .unwrap();
                    id += 1;
                }
            }
        }
        transaction.commit().unwrap();
        let seconds = started.elapsed().as_secs_f64();
        println!(
            "PROBE batch={batch_size}: {:.0} rows/s ({} rows in {seconds:.2}s)",
            total as f64 / seconds,
            total
        );
        connection.execute("DELETE FROM nodes", []).unwrap();
    }
}

#[test]
#[ignore = "real-data probe v3: store-layer load_revision end to end"]
fn store_load_revision_end_to_end() {
    use std::time::Instant;
    let store = diskgraph_store::SqliteSnapshotStore::open(std::path::Path::new(DB)).unwrap();
    let started = Instant::now();
    let graph = store
        .load_revision("rev-c3bffcb9-e66b-430f-bc67-8bae3a24dd4c")
        .unwrap();
    let seconds = started.elapsed().as_secs_f64();
    println!(
        "PROBE store.load_revision: {seconds:.2}s for {} nodes",
        graph.nodes.len()
    );
}

#[test]
#[ignore = "real-data probe v4: isolate the pragma overhead"]
fn pragma_overhead_isolation() {
    use std::time::Instant;
    for (label, with_pragma) in [("plain", false), ("wal+mmap", true)] {
        let connection = rusqlite::Connection::open(DB).unwrap();
        if with_pragma {
            connection
                .pragma_update(None, "journal_mode", "WAL")
                .unwrap();
            connection
                .pragma_update(None, "synchronous", "NORMAL")
                .unwrap();
            connection
                .pragma_update(None, "mmap_size", 1_i64 << 31)
                .unwrap();
        }
        let snapshot_id: String = connection
            .query_row(
                "SELECT snapshot_id FROM graph_revisions WHERE revision_id = ?1",
                ["rev-c3bffcb9-e66b-430f-bc67-8bae3a24dd4c"],
                |row| row.get(0),
            )
            .unwrap();
        let started = Instant::now();
        let mut statement = connection
            .prepare("SELECT node_json FROM nodes WHERE snapshot_id = ?1 ORDER BY id")
            .unwrap();
        let count = statement
            .query_map([&snapshot_id], |row| row.get::<_, String>(0))
            .unwrap()
            .count();
        let seconds = started.elapsed().as_secs_f64();
        println!("PROBE read-only scan [{label}]: {seconds:.2}s ({count} rows)");
    }
}
