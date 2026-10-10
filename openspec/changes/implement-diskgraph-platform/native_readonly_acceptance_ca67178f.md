# ca67178f 桌面只读原生验收记录

范围：macOS/Linux/Windows 只读 CLI 与 MCP。源码提交 `ca67178f294d4abf1c64241245c7e431db41026e`；本记录不关闭移动端、GUI、写操作或整个 OpenSpec 变更。

## 当前 CI

[CI 38057448226](https://github.com/loong10k/diskgraph/actions/runs/38057448226) 首次执行终态为 22 成功、1 失败。唯一失败是 Intel job 的 GitHub ArtifactService 上传超时，导致随后 workspace、Clippy 和迁移门禁被跳过，不能将其算作通过。原始终态见 `docs/benchmarks/readonly_ci_ca67178f_2026_10_10/workflow_attempt_1_terminal.json`。

仅对该基础设施失败执行一次单 job 补验，仍使用同一源码：attempt 2，job `114239019628`。补验终态成功，最终 CI 23/23 success；没有改变生产代码、期限、默认测试调度或断言。GitHub 为沿用的 22 个成功执行分配了新的展示 job ID；其起止时刻和全部步骤时刻/结果逐项相同，未重复执行或累加计数。映射见 `docs/benchmarks/readonly_ci_ca67178f_2026_10_10/inherited_successful_execution_mapping.json`，终态见 `workflow_attempt_2_terminal.json`。原失败与补验分开保留；dispatch 见 `macos_intel_upload_failure/rerun_dispatch.json`。各早期 receipt 中的 in_progress/未完整验收字段是当时观察，不覆盖此最终终态。

## 五个原生包

全部完成 stdio 18、HTTP 14、升级/回滚 7、20k 负载 6、200k 负载 6 项检查；已校验下载 archive sidecar、包内 worker 字节数与 SHA256，并保留原 job 日志。

| 目标 | 200k 扫描秒 | 查询 p50 ms | 查询 p95 ms | 实际清理秒 | 数据库/WAL 文件字节 |
|---|---:|---:|---:|---:|---:|
| aarch64-unknown-linux-gnu | 19.082 | 6.702 | 9.21 | 5.875 | 594546688 |
| x86_64-unknown-linux-gnu | 27.484 | 15.266 | 23.076 | 28.79 | 594546688 |
| x86_64-pc-windows-msvc | 74.713 | 25.33 | 31.494 | 13.659 | 473374720 |
| aarch64-apple-darwin | 27.092 | 52.131 | 79.609 | 18.051 | 449581056 |
| x86_64-apple-darwin | 76.442 | 122.045 | 163.285 | 68.771 | 472477696 |

这是各自 runner 上的受控单轮观测，32 次查询、4 客户，不能跨硬件排列性能优劣或称为前后性能改善。文件字节不是分配卷空间、累计写入或峰值 WAL；当前包已补测 Linux ARM64 容器的 20ms 进程树 RSS：20k 为 33,020 KiB，200k 为 182,660 KiB，各 6/6 检查通过；二进制和夹具前后摘要不变、原容器实际移除。证据见 `docs/benchmarks/release_rss_ca67178f_2026_10_10/receipt.json`。这不是严格峰值上界、Windows/macOS RSS、配对改善或长期运行证明；首测漏传 worker 部署配置而返回 Unsupported 的失败也单独保留。GNU ELF 编译基线不替代旧 glibc 用户空间运行验收。

Intel 200k 本轮在原有整次 300 秒门禁内完成，包括最终目录实际删除；此前 1ef9cfed 清理阶段超时记录继续保留。没有降低文件数或提高期限，也不保证任意宿主负载都在 300 秒内完成。

原生包报告目录：

- `docs/benchmarks/linux_gnu_package_ca67178f_2026_10_10/`
- `docs/benchmarks/windows_readonly_package_ca67178f_2026_10_10/`
- `docs/benchmarks/macos_readonly_packages_ca67178f_2026_10_10/`

## 当前包深目录观测

Linux ARM64 同一隔离容器环境复用原 300 层夹具，仅替换执行/输出路径，保留全部断言。当前包精确 601 节点、最大深度 301；32 查询/4 客户的 revision 绑定与结果上限通过。扫描 0.1182 秒、查询 p50 14.5673ms/p95 18.9115ms、数据库/WAL 文件 3,117,056 字节。原容器已移除，二进制前后摘要一致。见 `docs/benchmarks/deep_release_ca67178f_2026_10_10/receipt.json`。这是单平台单轮观测，没有 deep RSS、配对改善或生产 SLO 证明。

## 已核验的实际原生测试

Linux 三个、macOS ARM64 两个、Windows 两个完整 Rust job 首次均成功；Intel 补验实际 foundation 968/0/17、Engine/入口 1525/0/22（通过/失败/忽略），Clippy 和迁移通过。测试数量分别保留 foundation 和 Engine/入口两段实际执行结果，不累加重复迁移、隔离夹具或测试清单；ignored 不算通过。见 `docs/benchmarks/readonly_ci_ca67178f_2026_10_10/*/receipt.json` 与原日志。

- macOS ARM64 stable/1.97 的真实 SIGINT、SIGTERM 进程测试实际通过。原端口预留/释放竞态已由子进程 port 0 原监听器接线消除；旧 pre-readiness 退出没有保留 stderr，不能倒推其具体原因。
- Windows stable/1.97 的新原生 junction 测试均实际通过：记录 junction 自身身份，不索引外部目标，清理链接保留外部文件。它不关闭全部原生路径/卷语义父任务。
- Windows stable 的隔离真实 CFAPI 提供方测试 1/1：产品 read/hash 不触发 FETCH_DATA，普通读取正控触发 FETCH_DATA，原提供方退出。允许明确 Placeholder 或 Unsupported，不代表可读取占位正文或所有商业 provider。
- Linux ARM64 私有 FUSE 原生拒绝合同 4/4：直接/嵌套 read/hash 不读取内容，普通读取正控实际读取，原挂载卸载、提供方退出；不冒充通用云提供方验收。

## 尚未关闭的生产门禁

1. 固定 50ms 终检授权观察窗口在此前原生运行出现连接打开 89.955ms、SQL 125.493ms 的失败；当前索引窄读与零 busy 等待没有证明消除此问题。能力回调、授权新鲜性和现行窗口均保持，方案见 `authorization_window_decision.md`，仍待用户决定，不靠补验转绿声称根因已修复。
2. 异常不可回收 I/O 下，目前等待原 owner 真实退休，不能保证前端有限退出。独立监督部署决策仍待用户决定；不通过丢弃句柄、假退出或缩减验收完成该项。
3. provider、性能及长期运行证据须按各自适用范围验收；上述 CFAPI/FUSE 受限拒绝结果不关闭更宽父任务。扫描取消仍受上游目录边界及并行任务影响，不承诺严格 RSS 上限。

结论：本轮已新增平台验收证据，尚不能宣布整体生产就绪；未修改任何任务勾选、核心验收标准或部署范围。
