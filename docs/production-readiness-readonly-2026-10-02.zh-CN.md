# 桌面只读生产就绪记录 — 2026-10-02

范围：macOS、Linux、Windows 的 CLI，以及 MCP stdio、现代 HTTP、legacy SSE。文件写工具继续关闭。验收事实源为 OpenSpec `implement-diskgraph-platform` 的 RE-06 与 Q-09。

| 门禁 | macOS arm64 | macOS x86_64 | Linux x86_64 | Windows x86_64 |
| --- | --- | --- | --- | --- |
| 当前源码测试、fmt、Clippy | 本机通过 | CI 已配置，待实际运行 | CI 已配置，待实际运行 | CI 已配置，待实际运行 |
| 实际 CLI/MCP 二进制协议验收 | 本机 release：stdio 11/11、HTTP/SSE 13/13 | 原生 release 任务待运行 | CI/release 待运行 | CI/release 待运行 |
| v7→v8 一致性备份迁移 | 隔离夹具通过 | CI 待运行 | CI 待运行 | CI 待运行 |
| 包制品摘要及升级/回滚演练 | 本地二进制摘要及旧版回滚 7/7；打包待完成 | 待完成 | 待完成 | 待完成 |
| 目标环境容量与并发运行观察 | 已有隔离 20k/200k 测量；部署演练待完成 | 待完成 | 待完成 | 待完成 |

本机证据：macOS arm64、Rust 1.98.1，HEAD `e4d6074` 加未提交修改。2026-10-02 `cargo test --workspace --all-targets --locked --quiet` 通过；真实宿主项目仍被忽略，release 20k/200k 夹具另行实际运行。`cargo clippy --workspace --all-targets --locked -- -D warnings` 通过。vendored 扫描器恢复为原固定字节后，摘要测试通过。`cargo build --release --locked -p diskgraph-cli -p diskgraph-mcp` 构建的两个二进制版本均为 0.3.0，并通过两套可移植验收脚本。当前 SHA-256：

| 二进制 | SHA-256 |
| --- | --- |
| `target/release/diskgraph` | `1b84d909d10b66eb1bf009ecfe85096d0c5cb4ac3421026008b98c94240dffb1` |
| `target/release/diskgraph-mcp` | `1e6ddf4896c01472ecc6d762eddfb13fdb5da18e3f3ae74ecdf08fe8459d5a0b` |

这些是未提交工作区生成的本地二进制摘要，不是可分发包的摘要；release 工作流会另行计算归档摘要。CI 矩阵在三种 OS 上构建并测试真实二进制，再运行隔离 CLI/stdio 与签名 token HTTP/SSE 验收。监听服务从受限文件读取验证密钥，随附的 systemd 单元不再把密钥放入进程参数。原生 release 任务（含独立的 macOS Intel runner）在打包前重复协议验收。交叉构建的 Linux arm64 制品仍需在相应宿主运行，才能宣称该架构就绪。

[release 原始基准数据](benchmarks/readiness-2026-10-02.json)使用独立隔离进程、32 字节文件，并明确注入一条根目录可重建证据。正目标候选查询在 20k 文件时，完整加载与窄读的 p95 分别为 12.38 ms、0.35 ms；在 200k 文件时分别为 106.04 ms、0.36 ms；300 层深目录为 0.54 ms、0.48 ms。窄读采样 11 次、完整加载采样 3 次，属于热缓存观察值，不是 SLA。扫描阶段峰值 RSS 在 20k 时约 37 MB、200k 时约 225 MB；进程全程峰值包含完整加载对照，不能解释为窄读 RSS。数据库文件分别约 31 MB、316 MB。扫描器仍没有严格 RSS 上限。

本地复验：构建 CLI 与 MCP 后执行 `python scripts/accept-readonly-stdio.py`、`python scripts/accept-readonly-http.py`；设置 `DISKGRAPH_ACCEPT_BIN_DIR` 可指定 release 二进制目录。脚本使用临时数据目录、测试 scope、短期 token 和仅含 `metadata:read` 的授权，并验证匿名、恶意 Origin、无授权及撤权请求被拒绝；不访问生产控制库。

图库 schema 8 增加候选大小和证据关系索引。升级前以 SQLite 一致性机制备份到 `migration_backups/`；v7→v8 测试核对已提交数据及两个新索引。另一项 macOS 二进制演练在不切换分支的前提下归档 HEAD `e4d6074`，隔离构建其 schema 4 CLI，运行 `python scripts/accept-readonly-upgrade.py --old-cli OLD --new-cli NEW`：7/7 通过，涵盖 revision/树保留、双库备份、旧二进制拒绝升级库、恢复两库后可由旧二进制读取。运行回滚时，应先停止 CLI/MCP 与 job worker，保留升级失败的数据目录作为证据，将图库和控制库从同一升级前备份集恢复，再使用旧二进制打开恢复后的目录。旧二进制不得直接打开新 schema。此流程仍需在各目标 OS 以实际打包制品演练。图库 WAL/NORMAL 在断电时可能丢失最近的可重建索引提交；控制库 FULL 保护自身事务，两库没有跨库原子性保证。

当前结论：**尚未达到三平台生产就绪**。配置好的工作流与本机 macOS 通过不能代替 Linux/Windows 运行结果、归档校验、回滚及受控环境运行观察。
