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

## 独立读取配置成本诊断（不修改生产）

Store ignored release 诊断对隔离真实迁移库进行100样本串行配置分阶段测量；原configured与故意缺少安全配置的raw总耗时p50分别0.216958/0.214209ms。cache_size阶段p50 0.164ms，但该阶段为首次依赖schema的操作，不能把耗时全归于设置缓存参数。未删除PRAGMA、deadline或新鲜授权观察；本机空库诊断不能关闭Linux200k约1.9倍查询退化。初次并行运行互相干扰，不纳入串行结果。证据 reader_configuration_phases_2026_10_07.json。

输出字段明确限定配置值/顺序，并标记admission未测；改标签后release串行复测configured/raw p50为0.276542/0.275500ms，仍未观察到删配置收益，两次结果均保留。诊断2/0、fmt/Clippy通过，双路APPROVE/CLEAR，schema归因保持推断。

## 普通权限入口回调与控制锁分离

Engine::require 在控制锁外取得请求能力决定，然后在锁内新鲜读取持久授权交集；回调期间撤权仍必须拒绝。原require_with_control保留已有guard调用语义，其他有界终检回调仍持锁，本项不宣称全Engine回调已无锁。50ms有限重入控制读取回归旧路径实际失败，修复后应能读取持久server；独立连接在回调内撤销grant、旧能力允许时最终必须PermissionDenied。不声明同步回调硬抢占或整体期限。

普通require验证：有限重入真实RED→GREEN，相关授权组3/0；source_layout6/0，fmt/Clippy通过，双路APPROVE/CLEAR。完整Engine538/98/13，98项均Unsupported。兼容边界：控制锁中毒等错误现在在外部回调之后观察，回调可能先执行；普通入口仍可能等待控制锁，成功后的独立连接撤权仍依赖调用方后续安全门禁。

## 内容终态授权的锁外回调

require_read_terminal 保留初次范围存在/撤销检查，释放控制锁后取得ContentRead能力决定，再用新鲜控制锁核对持久交集和live_permission（无策略可信兼容模式同样拒绝scope撤销）。权限专用测试不打开文件、不替代内容平台能力门禁：原有限重入控制读失败确认RED；callback期间grant撤销、scope撤销及无策略scope撤销均需PermissionDenied。现有真实read_bounded终态测试保留；本机不支持的原生读取不得改为跳过或通过。

内容终态本机验证：权限重入真实RED→GREEN，专用两项1/0+1/0（撤销测试内3模式）；source_layout6/0，fmt/Clippy与双路APPROVE/CLEAR。完整Engine540/98/13，98项均Unsupported。实际内容能力验收保持未完成，本项未赋予平台读取能力，控制等待仍非硬有界。
