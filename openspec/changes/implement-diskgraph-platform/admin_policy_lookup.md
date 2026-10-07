# 管理员单项授权读取优化

目标是消除 Engine 管理员权限检查每次重建全部 Grant/HashSet 的分配，保留权限能力与当前持久授权交集，不使用缓存，不替换 holds_admin 的恢复管理员规则。

新 policy_permission 在一次 SQLite 观察内读取类型化 policy(version,revoked)，并以逐行借用字段匹配精确 principal/permission/scope/current-version。无策略为None，撤销或version<=0为Some(false)；管理员伪scope不读取scope注册撤销，普通live_permission保持。

为保持旧全表异常拒绝，在有效策略时逐行借用SQLite缓冲区，检查字段类型与UTF-8，并解码i64版本。正常授权不在Rust重建所有Grant/String/HashSet。仍扫描全量grants，CPU/读取量不是O(1)，不称严格读量恒定。最初仅用typeof的候选被真实非法UTF-8 TEXT回归拒绝，未接入Engine；其旧性能记录不能用于证明当前候选。

验收：正常版本、旧/未来grant、精确身份不匹配与旧authorizer决定一致；无策略与撤销状态不误放行；独立连接撤权即时观察；无关损坏类型仍错误。release大策略配对测量只比较当前查询方法，不代替CLI/MCP端到端或原生平台验收。

首轮测试因新接口尚不存在编译失败，不记录为行为回归RED。实现后初始3项语义测试通过。Engine已接入借用实现，当前性能与独立审查证据见下文。

非法UTF-8 TEXT兼容回归确认真实RED（旧authorizer错误、新候选却允许），改为借用逐行验证后4项通过、1项手动基准忽略。基准分位数已改nearest-rank p50/p95，40样本索引19/37。

借用实现release配对测量（20,001grant、40组交替、热缓存隔离内存库）：旧重建 p50/p95 4.365/5.074 ms，新借用读取 2.605/2.995 ms。Engine管理员路径已接入；真实隔离独立连接撤销精确grant与非法UTF-8 TEXT 1项测试通过，原授权期限6项回归通过。指标不覆盖整次CLI/MCP请求或平台产物。性能数据见 admin_policy_borrowed_lookup_2026_10_07.json。

独立审查：APPROVE / CLEAR；借用验证修复了非法UTF-8回归，未改变普通scope或恢复管理员规则。

当前源码本机验证：Store 全量 lib 测试 309 通过、0 失败、3 忽略；Engine source_layout 6 通过；Store/Engine fmt 检查及 all-targets Clippy -D warnings 通过。未据此声明全平台或整个 Engine 验收通过。
