# 桌面只读生产就绪记录 — 2026-10-02

范围：macOS、Linux、Windows 的 CLI，以及 MCP stdio、现代 HTTP、legacy SSE。文件写工具继续关闭。验收事实源为 OpenSpec `implement-diskgraph-platform` 的 RE-06 与 Q-09。

| 门禁 | macOS arm64 | macOS x86_64 | Linux x86_64 | Windows x86_64 |
| --- | --- | --- | --- | --- |
| 当前源码测试、fmt、Clippy | 本机通过 | CI 已配置，待实际运行 | CI 已配置，待实际运行 | CI 已配置，待实际运行 |
| 实际 CLI/MCP 二进制协议验收 | 本机 release：stdio 11/11、HTTP/SSE 13/13 | 原生 release 任务待运行 | CI/release 待运行 | CI/release 待运行 |
| v7→v8 一致性备份迁移 | 隔离夹具通过 | CI 待运行 | CI 待运行 | CI 待运行 |
| 包制品摘要及升级/回滚演练 | 本地归档摘要及包内二进制回滚 7/7；CI 制品待运行 | 待完成 | 待完成 | 待完成 |
| 目标环境容量与并发运行观察 | 已有隔离 20k/200k 测量；部署演练待完成 | 待完成 | 待完成 | 待完成 |

本机证据：macOS arm64、Rust 1.98.1，二进制源码提交 `3a6eaae`。2026-10-02 `RUSTFLAGS='-D warnings' cargo test --workspace --all-targets --locked --quiet` 通过；真实宿主项目仍被忽略，release 20k/200k 夹具另行实际运行。`cargo clippy --workspace --all-targets --locked -- -D warnings` 与 vendored 摘要测试通过。升级 Ratatui 及其 `lru` 依赖后，`cargo audit --json` 未发现锁文件依赖漏洞。`cargo build --release --locked -p diskgraph-cli -p diskgraph-mcp` 构建的两个二进制版本均为 0.3.0，并通过两套可移植验收脚本。当前 SHA-256：

| 二进制 | SHA-256 |
| --- | --- |
| `target/release/diskgraph` | `5f3a2eb95d8ac2e0e4018759e5da2e0d8861985d5fd1fd6e63a5402f771267a2` |
| `target/release/diskgraph-mcp` | `7244c9f6dbaa234635295b10ecf6b5a8d894316bf857bf3a5eca0247c7b5c5f8` |

这些是本地二进制摘要，不是已发布版本的摘要。`scripts/accept-readonly-package.py` 生成的本地归档 SHA-256 为 `064f833bbbb7805d3bc21cf89665a0ed652b24b80d28f067fa54a5604a421531`；解包后的二进制通过 stdio 11/11、HTTP/SSE 13/13、升级/回滚 7/7。CI 在 macOS ARM/Intel、Linux 和 Windows 上构建测试并生成非 release 归档，对解包二进制重复上述验收。监听服务从受限文件读取验证密钥，随附的 systemd 单元不再把密钥放入进程参数。交叉构建的 Linux arm64 制品仍需在相应宿主运行，才能宣称该架构就绪。

[release 原始基准数据](benchmarks/readiness-2026-10-02.json)使用独立隔离进程、32 字节文件，并明确注入一条根目录可重建证据。正目标候选查询在 20k 文件时，完整加载与窄读的 p95 分别为 12.38 ms、0.35 ms；在 200k 文件时分别为 106.04 ms、0.36 ms；300 层深目录为 0.54 ms、0.48 ms。窄读采样 11 次、完整加载采样 3 次，属于热缓存观察值，不是 SLA。扫描阶段峰值 RSS 在 20k 时约 37 MB、200k 时约 225 MB；进程全程峰值包含完整加载对照，不能解释为窄读 RSS。数据库文件分别约 31 MB、316 MB。扫描器仍没有严格 RSS 上限。

本地复验：构建 CLI 与 MCP 后执行 `python scripts/accept-readonly-stdio.py`、`python scripts/accept-readonly-http.py`；设置 `DISKGRAPH_ACCEPT_BIN_DIR` 可指定 release 二进制目录。脚本使用临时数据目录、测试 scope、短期 token 和仅含 `metadata:read` 的授权，并验证匿名、恶意 Origin、无授权及撤权请求被拒绝；不访问生产控制库。

图库 schema 8 增加候选大小和证据关系索引。升级前以 SQLite 一致性机制备份到 `migration_backups/`；v7→v8 测试核对已提交数据及两个新索引。另一项 macOS 二进制演练在不切换分支的前提下归档 HEAD `e4d6074`，隔离构建其 schema 4 CLI，运行 `python scripts/accept-readonly-upgrade.py --old-cli OLD --new-cli NEW`：7/7 通过，涵盖 revision/树保留、双库备份、旧二进制拒绝升级库、恢复两库后可由旧二进制读取。运行回滚时，应先停止 CLI/MCP 与 job worker，保留升级失败的数据目录作为证据，将图库和控制库从同一升级前备份集恢复，再使用旧二进制打开恢复后的目录。旧二进制不得直接打开新 schema。此流程仍需在各目标 OS 以实际打包制品演练。图库 WAL/NORMAL 在断电时可能丢失最近的可重建索引提交；控制库 FULL 保护自身事务，两库没有跨库原子性保证。

当前结论：**尚未达到三平台生产就绪**。四种 OS 的原生制品工作流正在运行；本机 macOS 通过不能代替其结果或受控环境运行观察。Windows 内容读取及所有文件变更在原生句柄语义验收前继续返回不支持。
