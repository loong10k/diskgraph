//! D30 原生定位主键查询工作量回归。
use crate::native_locator_tests::{budget, located_store};

#[test]
fn native_locator_point_query_does_not_scan_or_decode_unrelated_nodes() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    for unrelated in [20_000, 200_000] {
        let store = located_store();
        store.connection.execute("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<?1)
        INSERT INTO nodes(snapshot_id,id,parent_id,locator_key,name,subtree_bytes,node_json,native_locator_kind,native_locator_encoding,native_locator_raw)
        SELECT 'located',x+2,1,'null','unrelated',0,'null','invalid','invalid',zeroblob(100) FROM n;", [unrelated]).unwrap();
        let steps = Arc::new(AtomicUsize::new(0));
        let observed = steps.clone();
        store
            .connection
            .progress_handler(
                1,
                Some(move || observed.fetch_add(1, Ordering::Relaxed) > 500),
            )
            .unwrap();
        let result = store
            .native_locator_bounded("located", 2, &mut budget(4096, 1))
            .unwrap()
            .unwrap();
        store
            .connection
            .progress_handler(0, None::<fn() -> bool>)
            .unwrap();
        assert!(result.locator.is_some());
        let work = steps.load(Ordering::Relaxed);
        eprintln!("D30 native locator: unrelated={unrelated}, selected=1, sqlite_vm_steps={work}");
        assert!(work < 500);
    }
}
