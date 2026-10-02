/// 发布 revision 与快照、时间的绑定记录。
/// 来源：DiskGraph 原生 Rust 存储设计；无 Java 对应实现。
/// One published graph revision bound to a snapshot.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RevisionRecord {
    pub revision_id: String,
    pub snapshot_id: String,
    pub published_at_unix_ms: u64,
}
