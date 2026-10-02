/// 完整加载路径的 SQLite 节点 tuple，保持原 16 列顺序和类型。
/// 来源：DiskGraph 原生 Rust 存储设计；无 Java 对应实现。
pub(crate) type FullNodeRow = (
    i64,
    Option<i64>,
    String,
    String,
    i64,
    String,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<String>,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<i64>,
);
