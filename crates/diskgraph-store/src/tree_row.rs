/// 不加载 locator/JSON 的树视图窄行，保持旧 tuple 字段顺序。
/// 来源：DiskGraph 原生 Rust 存储设计；无 Java 对应实现。
/// One narrow node row for the tree view: no locator, no JSON payload.
pub type TreeRow = (
    u64,
    Option<u64>,
    String,
    String,
    i64,
    i64,
    i64,
    i64,
    i64,
    Option<String>,
);
