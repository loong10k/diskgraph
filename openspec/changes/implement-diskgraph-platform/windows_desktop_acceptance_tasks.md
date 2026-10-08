# Windows 台式机原生验收任务

状态：执行中。2026-10-08 WebCodex-台式机连接已恢复，实际定位 `E:\workspaces\workspace-loong10k\diskgraph`，干净main从26b6f1ca快进到b3259315。Windows 11 build26200、i7-13700K、约32GiB内存、NTFS可用约111GiB；已有Rust stable1.99.0、MSVC2022、Python3.11.15、PowerShell5.1。精确Rust1.97.0、Python3.13、PowerShell7未发现，不静默安装或冒称已运行。正在绑定最终固定提交与原生验收材料。

范围：Windows x86_64 只读 CLI/MCP，与 macOS/Linux 工作并行。此清单延续 implement-diskgraph-platform，不另建规格体系。Windows 验收未完成不阻塞其他平台实现，但不得将总目标标记生产就绪。默认关闭危险写工具，不操作用户业务数据库或扫描真实全盘。

## WD-01 仓库与执行身份

- [x] 在台式机实际工作区中定位 `workspace-loong10k/diskgraph`，记录真实绝对路径、Git 根目录、分支、HEAD、origin、工作区状态及已有 AGENTS.md。
- [x] 检查本地未提交工作与活动构建；不得 reset、clean、stash、覆盖用户改动或更换现有分支。确认 origin 为 loong10k/diskgraph。满足快进条件后执行 `git fetch origin`、`git merge --ff-only origin/main`；不满足时保存差异并报告。
- [ ] 固定验证 SHA，开始和结束分别记录 HEAD、受测源文件摘要和产物 SHA256。运行期间不要再次更新源码。不得把旧提交日志与新提交混合为通过。
- [ ] 记录 Windows 版本、CPU、内存、卷格式/剩余空间、Python/MSVC/Rust 版本以及后台负载。创建独占临时验收目录，保留成功与失败的完整输出、退出码、开始/结束时间。

## WD-02 受信扫描夹具准备

已有依赖缺失时报告，不静默安装。建议 PowerShell 7、现有 MSVC x64 和 Python 3.13；分别核验 stable 与 Rust 1.97.0。以下相对路径都从真实仓库根目录执行。每条外部命令必须检查 `$LASTEXITCODE`；非零不得继续称前置成功。

```powershell
$dgEvidence = Join-Path $env:TEMP ("diskgraph-desktop-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $dgEvidence | Out-Null
$env:RUNNER_TEMP = $dgEvidence
$env:GITHUB_ENV = Join-Path $dgEvidence 'acceptance.env'
$env:GITHUB_SHA = (git rev-parse HEAD).Trim()
$env:DISKGRAPH_QUERY_DIAGNOSTICS = '1'
$env:RUSTFLAGS = '-D warnings'
Start-Transcript -Path (Join-Path $dgEvidence 'session.log')

cargo build --workspace --all-targets --locked
# 在检查上一条退出码后继续。
./scripts/configure_windows_exit_fixtures.ps1
cargo build -p diskgraph-scan-worker --locked --message-format=json > "$dgEvidence/worker-build.jsonl"
./scripts/configure_windows_acceptance_worker.ps1 -Artifacts "$dgEvidence/worker-build.jsonl" -OutputDir $dgEvidence

# CI 的 GITHUB_ENV 在本地不会自动载入，必须在当前进程显式应用。
Get-Content -LiteralPath $env:GITHUB_ENV | ForEach-Object {
    if ($_ -match '^([A-Z][A-Z0-9_]*)=(.*)$') {
        [Environment]::SetEnvironmentVariable($matches[1], $matches[2], 'Process')
    }
}
```

- [ ] 保存 Cargo JSON、product-worker-receipt.json、退出码夹具摘要。扫描程序必须来自固定 SHA 的真实构建，不从邻接清单自行建立信任。
- [ ] 重新跑另一 toolchain 或 release 时使用新的独占 output-dir；现有脚本的 CreateNew 不能以覆盖文件绕过。

## WD-03 已知撤权与期限回归（优先）

```powershell
cargo test -p diskgraph-engine --lib --locked history_relation_withdrawal_tests -- --nocapture
cargo test -p diskgraph-engine --lib --locked terminal_capability_tests -- --nocapture
cargo test -p diskgraph-engine --lib --locked relation_request_tests -- --nocapture
```

- [ ] `committed_withdrawal_survives_terminal_sql_expiry` 实际执行，Windows 原生 watch 必须存在；关系/历史已撤销并重授后，原 50ms SQL 窗口过期仍为 PermissionDenied，不能进入编码。
- [ ] `terminal_sql_expiry_without_withdrawal_remains_budget_exceeded` 实际执行；无撤权时保持 BudgetExceeded，不能批量映射为拒权。
- [ ] 原 CI 失败 `history_and_relation_remember_withdrawal_in_capability_observations` 必须通过；能力到期、双侧撤权、原连接争用、revision 隔离及编码前后末检继续通过。不得提高额度、重试到绿或删改断言。

## WD-04 完整源码与原生安全验收

```powershell
cargo test --workspace --all-targets --locked --no-fail-fast -- --nocapture
cargo test -p diskgraph-store --lib --locked -- --test-threads=1
cargo clippy --workspace --all-targets --locked -- -D warnings
python -m unittest discover -s scripts/tests -v
python scripts/test_scan_worker_package.py -v
python scripts/test_worker_manifest_admission.py -v
python -m unittest discover -s scripts/tests -p test_windows_acceptance_job.py -v
cargo test -p diskgraph-engine --test content_cloud_windows --locked -- --nocapture --test-threads=1
```

- [ ] stable 与 1.97.0 分别记录完整测试结果；换 toolchain 使用 `cargo +<toolchain>` 并重新构建、绑定该组 worker，不能只修改版本标签。
- [ ] Cloud Files 测试必须 1 passed / 0 ignored，并出现 `DG_REAL_CFAPI_PRODUCT_NO_FETCH_AND_ORDINARY_FETCH_CONTROL=1`；产品 fetch=0、普通读正控 fetch>0。
- [ ] 路径/卷/128位 file ID、reparse point、根与父目录替换、并发修改、未知大小、历史身份不足拒绝和非 UTF-8 兼容语义均检查实际 Windows 测试输出。
- [ ] 真实 Job/后代退休、失败 owner 保留、取消/撤权/租约 fence、不发布部分扫描和数据库迁移/回收保护检查实际执行数量，不把零测试或 ignored 算成功。
- [ ] 固定期限的隔离诊断可以保留，但不能替代原并发 workspace 失败。单测日志中的预期 panic 与实际 FAILED 应区分。

## WD-05 Release CLI/MCP 与包验收

按 `.github/workflows/ci.yml` 的 readonly-package Windows 配方，以当前 SHA 构建：

```powershell
cargo build --release --locked -p diskgraph-cli -p diskgraph-mcp -p diskgraph-scan-worker --target x86_64-pc-windows-msvc
```

- [ ] 用新的独占目录和对应 release Cargo artifact 再执行 configure_windows_acceptance_worker.ps1，并载入其环境；不得沿用 debug worker。
- [ ] 从固定旧提交 `e4d6074df496dd0816abca531362cdc68e082510` 的独立源码目录构建旧版 release CLI，不改变当前分支/checkout；记录旧产物摘要。
- [ ] 执行现有正式包验收，将 `--old-cli` 指向上一步真实产物，`--output-dir` 指向该组独占证据目录：

```powershell
python scripts/accept-readonly-package.py --target x86_64-pc-windows-msvc --bin-dir target/x86_64-pc-windows-msvc/release --old-cli <旧版CLI绝对路径> --output-dir <独占证据目录>
```

- [ ] 检查 stdio、真实认证 HTTP/Origin/SSE、60秒 HTTP 持续请求、升级/回滚、20k 与 200k 负载全部成功，保留归档包和摘要。
- [ ] HTTP 修复不刷新原60秒期限；已派发请求超时仍失败。macOS Intel 旧 explain 失败原因未知，Windows 如遇同类失败须保留新增 dispatch diagnostics，不能猜测根因。

## WD-06 Windows 200k 性能门槛

- [ ] 正式验收沿用 accept-readonly-package.py 的 300 秒外层期限，覆盖造数据、扫描、查询和清理。文件数必须 200000、节点数 200001，32 查询/4 客户端及 6/6 行为校验不减少。
- [ ] 记录造数据、扫描、查询 p50/p95、清理、全流程、数据库/WAL 增量和硬件/后台负载。旧 d8d3995f 失败后诊断为 670.766 秒（252.347 + 164.821 + 249.023 等），不是干净配对基线。
- [ ] 如果正式流程失败，可另外运行 `python scripts/diagnose_windows_load.py --bin-dir target/x86_64-pc-windows-msvc/release --output-dir <新的独占目录>`。该脚本900秒诊断结果只能用于定位，不能替代300秒通过；clean_environment_confirmed=false 不改写为 true。
- [ ] 内存峰值和更长 soak 单独记录。60秒 smoke 不代表长期稳定；单台桌面通过不代表所有硬件或严格 RSS 上限。

## WD-07 回传与关闭规则

- [ ] 保存每项命令、环境、固定 SHA、二进制摘要、原始 stdout/stderr、退出码、测试数量、跳过原因以及全部 JSON/receipt；生成文件摘要清单。
- [ ] 逐项标记 passed / failed / not_run，报告首个业务失败与后续环境失败。缺权限、provider、toolchain 或原生部署材料均为未验收，不静默降级。
- [ ] 结束时核对源码未变化、没有未回收的本任务进程；失败保留目录供诊断，不擅自终止其他任务或删除用户文件。
- [ ] 将证据回传当前任务，只有精确对应当前源码的成功项才勾选。Windows 任务本身完成不等于全平台生产就绪；其他平台、监督恢复和归属采集仍独立验收。
