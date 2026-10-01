# 桌面只读生产就绪记录 — 2026-10-02

macOS arm64/x86_64、Linux x86_64、Windows x86_64 的**只读 CLI 与 MCP stdio、现代 HTTP、legacy SSE 原生代码及制品门禁已通过**，可进入受控生产部署。二进制尚未签名、发布，也未连接生产数据目录运行。文件写工具继续关闭；Windows 内容读取仍不支持。本轮范围与门禁以 OpenSpec `implement-diskgraph-platform` 的 RE-06、Q-09 为准。

[CI 运行 36913827691](https://github.com/loong10k/diskgraph/actions/runs/36913827691) 在源码提交 `3d831bc3f467d9bbfb54d4cba5a17a0cb67444a2` 上的 13 个任务全部通过：rustfmt、vendored 扫描器校验、stable 与 Rust 1.97 workspace 矩阵、四种原生 release 配置制品。每个平台的解包二进制都通过 stdio 11/11、带认证的 HTTP/SSE 13/13、隔离升级/回滚 7/7，以及 20k 文件并发运行 4/4。协议测试使用有效签名 token、数据库授权和隔离目录，覆盖匿名、无授权、撤权、恶意 Origin 拒绝。Windows 还验证了 `Everyone` 可读的验证密钥必须被拒绝。打包脚本核对解包二进制与原构建二进制的摘要；下表归档 SHA-256 另与下载的 `.sha256` 文件逐一核对。[原始原生测量](benchmarks/readiness-native-2026-10-02.json)保留了精确数值和检查项。

| 原生目标 | 归档 SHA-256 | 20k 扫描 | 32 次冷 CLI 查询、4 客户端 p50 / p95 | 数据库 + WAL |
| --- | --- | ---: | ---: | ---: |
| macOS arm64 | `9562abe7ae3248aa2a5ed3a9ab65d891079163107e450fd78115c1c892b7312a` | 0.697 秒 | 39 / 200 毫秒 | 35.9 MB |
| macOS x86_64 | `9519634b9ce5dd3f25d748e13fe4a18e77cdf5e6ca19eea31667c4e3faa3b540` | 1.829 秒 | 57 / 673 毫秒 | 35.9 MB |
| Linux x86_64 | `3a4310805e5282913b089c3e8fd4978546d2d47d350c5a40c64747b8b978225e` | 0.633 秒 | 17 / 246 毫秒 | 29.9 MB |
| Windows x86_64 | `87f09efb60904bdf55e03fb4f16e48e8feb5215768675409f79a70079eb33b01` | 4.126 秒 | 438 / 2852 毫秒 | 33.3 MB |

四个制品版本均为 `diskgraph 0.3.0`；四次负载验收均发布完整 revision、将并发读取绑定至该 revision、限制响应，并在节点预算不足时拒绝部分发布。这些是不同 CI 宿主的单次观察值，既不是 SLA，也不能直接用来比较操作系统速度。Windows 四客户端冷 CLI 查询 p95 为 2.85 秒；延迟敏感场景仍需在目标宿主制定 SLO 并实测。该原生矩阵未测每个平台的峰值 RSS。

独立的[本机 release 基准数据](benchmarks/readiness-2026-10-02.json)使用 macOS arm64 隔离夹具和 32 字节文件。正目标候选查询在 20k 文件时，完整加载与窄读的 p95 分别为 12.38 毫秒、0.35 毫秒；200k 文件时分别为 106.04 毫秒、0.36 毫秒。这是热缓存采样（完整加载 3 次、窄读 11 次），不是 SLO。扫描阶段峰值 RSS 在 20k 时约 37 MB、200k 时约 225 MB；数据库文件分别约 31 MB、316 MB。扫描器仍无严格 RSS 上限。本机 `RUSTFLAGS='-D warnings' cargo test --workspace --all-targets --locked --no-fail-fast --quiet`、workspace Clippy `-D warnings`、workspace 包定向 fmt、vendored 摘要校验及 `cargo audit --json` 均通过，审计未发现有漏洞的锁文件依赖。

图库 schema 8 增加候选大小和证据关系索引。升级前以 SQLite 一致性机制将图库与控制库备份至 `migration_backups/`；v7→v8 夹具核对已提交数据及两个索引。四个原生制品任务均隔离构建旧版 schema-4 CLI，并对解包二进制执行 7 项升级/回滚演练。实际回滚须先停止 CLI/MCP 与 job worker，保留失败的升级目录，从**同一套升级前备份**恢复两个数据库，再用旧二进制打开恢复目录；不可让旧二进制直接打开新 schema。图库 WAL/NORMAL 在断电时可能丢失最近的可重建索引提交；控制库 FULL 保护自身事务，两库没有跨库原子性。

复验完整制品门禁：在隔离 checkout 构建新旧二进制后执行 `python scripts/accept-readonly-package.py --target TARGET --bin-dir RELEASE_BIN_DIR --old-cli OLD_CLI`。只复验协议时可执行 `python scripts/accept-readonly-stdio.py` 和 `python scripts/accept-readonly-http.py`，用 `DISKGRAPH_ACCEPT_BIN_DIR` 指定待测二进制。脚本使用临时图库与控制库，不触碰生产数据。监听服务从受限文件读取验证密钥，随附 systemd 单元不把密钥放入进程参数。

剩余边界属于实际运维：尚未签名发布，也未在生产宿主长期运行；原生 CI 不能代替部署现场的备份、容量、延迟及监控验收。Linux arm64 配有交叉构建工作流，但本次 CI 没有 arm64 Linux 原生验收。真实宿主文件操作演练不在常规 CI 内。Windows 内容读取及所有文件变更仍不支持，并保持关闭。上游扫描器可能在目录边界超过取消预算后短暂继续工作，因此不宣称严格峰值内存控制。更广的 OpenSpec 13.6 仍有宽目录精确统计、历史比较工作量观察及 TUI 页内排序展示待完善，不计入本轮限定的制品门禁。
