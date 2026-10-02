//! 搜索续页必须 seek 到 name/id，而非从快照开头重新匹配前置行。

use std::sync::atomic::Ordering;

use crate::child_aggregate_tests::{count_steps, wide_store};

#[test]
fn deep_search_keyset_does_not_revisit_preceding_matches() {
    let store = wide_store(false);
    store
        .connection
        .execute(
            "INSERT INTO node_search(snapshot_id,id,name_fold,path_fold)
         SELECT snapshot_id,id,name,name FROM nodes WHERE snapshot_id='wide' AND id>2",
            [],
        )
        .unwrap();
    let steps = count_steps(&store);
    let (items, more) = store
        .search_page("wide", "", Some(("child-000199998", 200000)), 199_998, 1)
        .unwrap();
    assert_eq!(items[0].id, 200001);
    assert!(more);
    assert!(
        steps.load(Ordering::Relaxed) < 2000,
        "search keyset rescanned preceding matches: {} VM steps",
        steps.load(Ordering::Relaxed)
    );
}
