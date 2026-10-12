# 控制授权查询编译开销优化

## 变更

固定权限、scope 撤销、授权代次、server 身份和策略状态/版本查询使用 rusqlite 的连接级有界编译缓存，每次重新绑定所有参数并执行原 SQL。没有缓存授权结果，没有跨调用读事务，不改变控制库 FULL、原 50ms 观察窗口、SQLite progress handler 或忙等待行为。归还语句时 rusqlite 清除参数绑定。

显式启用既有 rusqlite 0.40.2 的 cache 特性；Cargo.lock 仅新增可选依赖 hashlink 0.12.2 和 rusqlite 对它的引用，既有 hashbrown 0.17.1 复用，没有更新其他包或 vendored 源码/pin。原公共方法、错误语义和数据库 schema 保持兼容。

## 验证

最初探针因 cache 特性未启用和测试夹具方法名错误而编译失败，单独保存为 initial_compile_probe，不计作行为 RED。修正测试配置后，实际 SQLite authorizer 观测到原实现 101 轮共707次 SELECT 编译事件，热身7次，重复编译断言失败；另两个行为测试通过。实施后热身之外100轮无新增编译，3项通过。

跨独立真实控制连接提交 grant 撤销、重授、epoch 变更、scope 撤销后，已热身语句仍读取最新结果；换主体/权限不会复用旧允许。已热身语句仍执行实际 SQLite VM，被 progress handler 中断后恢复正常；原期限已过时拒绝。既有损坏策略、缺失范围和迁移回归包含在 Store 完整运行中。

macOS 当前工作区 Store：347通过、0失败、7忽略；新增 release 性能测试已单独显式执行1/1。Engine 末段能力14/14、关系/历史撤权6/6。Store/Engine all-target Clippy、项目10包格式检查、OpenSpec严格校验、上游pin/摘要4/4通过。工作区既有 lib.rs 和 reader_observation_tests 修改未纳入本变更；receipt 记录其测试上下文摘要，不能把该完整工作区测试清单冒称某个已提交树的逐项清单。

## Release 对照

同一个 macOS release 测试进程、内存SQLite、同样六次授权读取和原期限保护，采用 cache容量0/16/16/0 的 AB/BA 顺序，每组2000轮：

| 顺序 | 缓存容量 | p50 | p95 |
|---|---:|---:|---:|
| A1 | 0 | 19.875 微秒 | 21.959 微秒 |
| B1 | 16 | 2.875 微秒 | 3.000 微秒 |
| B2 | 16 | 2.875 微秒 | 3.042 微秒 |
| A2 | 0 | 20.208 微秒 | 21.375 微秒 |

容量0是同源码禁用编译缓存的对照，不是旧发布二进制。该结果只说明小型控制SQL的重复编译成本，不能推导CLI/MCP整体加速倍数、200k扫描改善、RSS或跨平台通过；也不证明 macOS Intel explain 旧失败已经修复。Linux/Windows和同SHA全平台门禁仍待验证，未勾选整体生产任务。

证据及源码摘要：`docs/benchmarks/control_query_compilation_2026_10_08/receipt.json`，所有初始失败、行为RED/GREEN、完整回归与测量日志均压缩保留。

## Linux ARM64 固定提交补充验收

将提交 `d4561895df3955eaa18104dac1e5471ea651c5fc` 的源码导出后，在固定摘要的 Rust 1.97.0 Debian trixie 容器执行，配额2 CPU/4GiB、用户501:20，未挂载真实业务数据库。Store 345通过、0失败、7忽略（156.17秒）；该清单来自已提交树，不包含 macOS 工作区另有的2项未提交测试。release 微基准1/1通过。

同样0/16/16/0容量、每组2000轮：p50分别19.542/3.333/3.375/19.417微秒，p95分别30.083/3.458/3.458/22.667微秒。其范围仍仅为内存SQLite控制SQL编译开销；不代表原生文件系统采集、全量Engine、Windows、macOS Intel、200k整流程或同SHA全平台就绪。源码归档、镜像摘要、脚本和日志摘要见 `docs/benchmarks/linux_control_query_d4561895_2026_10_08/receipt.json`。

## 2026-10-12 scope 能力的单次持久观察

新增 `ControlStore::policy_scope_permission`，Engine 的普通 scope 能力交集由原 policy_state→live_permission 两次读取改为一条参数化 SQL。策略、scope 和 grant 共享一次 WAL 观察；没有允许结果缓存或跨请求事务。管理员兼容入口和已有 live_permission API 不变。未发布策略仍返回 None，即使 scope 缺失；发布后的缺失 scope 拒绝，损坏策略必须先于 scope 缺失/撤销返回真实解码错误。保留原版本比较语义，包括损坏库防御回归，不修改 schema 或授权窗口。

用旧组合构造新接口的初始基线后，SQLite authorizer 观测到3个实际 SELECT 编译事件（顶层策略、顶层 scope 与 EXISTS），单次观察断言失败，另3项兼容回归通过；首次测试错误引用 Sql 而不是 Sqlite 造成的编译失败不计行为 RED。实施后顶层 SELECT 与 EXISTS 共2个编译事件。新增5项运行回归覆盖精确主体/权限/版本、未发布、撤销、损坏策略的错误顺序、独立 WAL 连接提交撤权以及真实 VM 中断/原期限，不把编译缓存当作授权缓存。

本机工作区 Store 全量385通过、8忽略，包含既有未提交 reader_observation_tests 的2项；不能把该清单冒称提交树完整清单。Engine scope 实际显示/撤权5通过，末段能力14通过；关系回归9通过、8因未安装受信 macOS 宿主而在 Engine::open 返回 Unsupported，未安装桌面服务。Store/Engine all-target Clippy、Engine布局6、Store布局、显式相关文件fmt和上游pin/摘要4项检查已执行；受限关系项、完整 workspace 与新源码目标平台验收仍交由后续CI，父项开放。

同进程 release、隔离内存SQLite、两种路径各自热身100次后2000样本，按旧/新/新/旧 AB/BA：p50为709/542/500/709ns，p95为792/1375/584/792ns。仅能说明单次授权工作减少和本机中位数变化，尾延迟结果不一致，不能称原生50ms或250ms窗口失败已解决，更不能外推整个工具性能。

日志与候选源码摘要见 `docs/benchmarks/policy_scope_atomic_2026_10_12/receipt.json`；原查询预算、数据库250ms及能力50ms均保持。
