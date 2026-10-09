# 终检同轮 SQL 编译复用

沿用 SC-04/Q-08。只在每轮新建的 `RevisionOwnershipReader` 内复用单条归属判断语句的编译产物；不得缓存允许/拒绝结果、复用上一轮连接或启动读事务。所有参数重新绑定，每次执行独立观察当前 WAL；原期限、取消和数据库替换识别保持不变。

验收：通过真实 SQLite authorizer 观察首次编译，后续不同参数读取不重复编译；独立写连接提交隔离后，同一编译产物立即拒绝；授权视图删除或变更触发错误或新语义，不能返回旧允许结果。既有旧 WAL 快照、数据库同路径替换、原 SQL 中断和 busy 剩余期限回归继续通过。先记录原实现 RED，再实现并验证；此项不关闭 Windows 默认回归、macOS Intel 授权期限或性能父门禁。

实现：每轮独立连接的 prepared statement cache 容量为 1，执行后归还已 reset 的语句；连接本身仍每轮新建。未调整生产期限、busy 行为、视图、权限或传输语义。

验证：macOS 和 Windows 均先取得真实 RED 1/1，失败为四次查询的 SQLite 编译观察数 16 而非首次的 4；实现后两平台归属回归均为 9/0/1（忽略项为 release 基准）。macOS Store 完整回归 362/0/8，Windows Store 完整回归 374/0/8；Windows 历史兼容矩阵 10/0/0、尺寸资格 9/0/0。Store 全目标严格 Clippy 与本次源码定向 rustfmt 检查通过。

macOS 历史集成的首次命令误用 library 过滤，运行 0 项，不能算通过；随后正确集成目标在创建原生引擎时因缺少受信宿主部署返回 Unsupported，兼容矩阵 0/10，Cargo 停止后未执行尺寸目标。没有安装特权宿主，也没有伪造托管 CI 环境；该本机验收仍缺失。一次离线构建缺少已锁定依赖，补取 Cargo.lock 固定依赖后编译完成，环境失败不计行为 RED。

另行在未经本优化的 `82e692d4` 上启用诊断运行 Windows 默认 CLI：86/2/0，仍报告 `terminal_ownership_sql` 原窗口耗尽。该结果与此前 82/6/0 的波动说明不能由单轮失败数推导修复收益。此项只证明同轮重复编译消除与列出的正确性回归；没有宣称 Windows 完整 CLI/MCP、macOS Intel 或 200k 性能门禁已修复。Windows 原始源码/程序摘要、日志摘要及回执见 `docs/benchmarks/windows_ownership_cache_{red,green}_82e692d4_2026_10_09.json`、`windows_store_full_ownership_cache_82e692d4_2026_10_09.json`、`windows_history_ownership_cache_82e692d4_2026_10_09.json`。

状态：本项实现与上述验证完成，平台生产父项保持开放。当前三平台 CI 绑定 `82e692d4`，尚不包含本优化。
