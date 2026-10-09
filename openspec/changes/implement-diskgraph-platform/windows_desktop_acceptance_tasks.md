# Windows 台式机原生验收任务

状态：执行中。2026-10-08 WebCodex-台式机连接已恢复，实际定位 `E:\workspaces\workspace-loong10k\diskgraph`，干净main快进到1ac266a0。Windows 11 build26200、i7-13700K、约32GiB内存、NTFS可用约111GiB；已有Rust stable1.99.0、MSVC2022、PowerShell5.1。PATH默认Python3.11.15，进一步找到已有uv Python3.13.12并明确使用；精确Rust1.97.0和PowerShell7未发现，不静默安装。完整失败与通过结果见 windows_desktop_native_2026_10_08.md，当前不满足全部门禁。

范围：Windows x86_64 只读 CLI/MCP，与 macOS/Linux 工作并行。此清单延续 implement-diskgraph-platform，不另建规格体系。Windows 验收未完成不阻塞其他平台实现，但不得将总目标标记生产就绪。默认关闭危险写工具，不操作用户业务数据库或扫描真实全盘。

## WD-01 仓库与执行身份

- [x] 在台式机实际工作区中定位 `workspace-loong10k/diskgraph`，记录真实绝对路径、Git 根目录、分支、HEAD、origin、工作区状态及已有 AGENTS.md。
- [x] 检查本地未提交工作与活动构建；不得 reset、clean、stash、覆盖用户改动或更换现有分支。确认 origin 为 loong10k/diskgraph。满足快进条件后执行 `git fetch origin`、`git merge --ff-only origin/main`；不满足时保存差异并报告。
- [ ] 固定验证 SHA，开始和结束分别记录 HEAD、受测源文件摘要和产物 SHA256。运行期间不要再次更新源码。不得把旧提交日志与新提交混合为通过。
- [ ] 记录 Windows 版本、CPU、内存、卷格式/剩余空间、Python/MSVC/Rust 版本以及后台负载。创建独占临时验收目录，保留成功与失败的完整输出、退出码、开始/结束时间。

## WD-02 受信扫描夹具准备

已有依赖缺失时报告，不静默安装。建议 PowerShell 7、现有 MSVC x64 和 Python 3.13；分别核验 stable 与 Rust 1.97.0。以下相对路径都从真实仓库根目录执行。每条外部命令必须检查 `$LASTEXITCODE`；非零不得继续称前置成功。

台式机现有Python3.13.12位于 `%APPDATA%\uv\python\cpython-3.13.12-windows-x86_64-none\python.exe`。脚本中的 `python` 必须绑定经实际版本检查的解释器；不能直接沿用 Espressif 的 PATH 默认解释器。符号链接攻击夹具需要创建链接权限，本机当前 WinError 1314 属于未验收，不能忽略失败或改成通过。

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

# 驱动测试需要独立协议夹具；产品 worker 不能代替它。与 CI 绑定同一 Cargo example。
# 由 Python 写入 Cargo 原始字节，避免 PowerShell 5.1 的重定向产生 UTF-16。
@'
import hashlib, json, os, subprocess
from pathlib import Path
output = Path(os.environ['RUNNER_TEMP'])
with (output / 'protocol-driver-build.jsonl').open('wb') as stream:
    subprocess.run(['cargo', 'build', '-p', 'diskgraph-engine', '--example',
                    'scan_worker_driver_fixture', '--locked', '--message-format=json'],
                   stdout=stream, check=True)
rows = [json.loads(line) for line in (output / 'protocol-driver-build.jsonl').read_text(encoding='utf-8').splitlines()]
artifacts = [row for row in rows if row.get('reason') == 'compiler-artifact'
             and row.get('target', {}).get('name') == 'scan_worker_driver_fixture'
             and row.get('target', {}).get('kind') == ['example'] and row.get('executable')]
if len(artifacts) != 1:
    raise SystemExit('one actual Cargo protocol-driver artifact is required')
image = Path(artifacts[0]['executable'])
if not image.is_absolute() or image.is_symlink() or not image.is_file() or any(c in str(image) for c in '\r\n\0'):
    raise SystemExit('invalid protocol-driver artifact path')
with image.open('rb') as stream:
    digest = hashlib.file_digest(stream, 'sha256').hexdigest()
receipt = {'checkout_sha': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
           'fixture_sha256': digest, 'fixture_bytes': image.stat().st_size,
           'fixture_source_sha256': hashlib.sha256(Path('crates/diskgraph-engine/tests/fixtures/scan_worker_driver_fixture.rs').read_bytes()).hexdigest(),
           'product_scan_image': False, 'production_acceptance': False}
(output / 'protocol-driver-receipt.json').write_text(json.dumps(receipt, indent=2) + '\n', encoding='utf-8')
with Path(os.environ['GITHUB_ENV']).open('a', encoding='utf-8') as stream:
    stream.write('DISKGRAPH_SCAN_DRIVER_FIXTURE=' + str(image) + '\n')
'@ | python -
if ($LASTEXITCODE -ne 0) { throw 'Protocol-driver fixture preparation failed' }

# CI 的 GITHUB_ENV 在本地不会自动载入，必须在当前进程按 UTF-8 显式应用。
Get-Content -LiteralPath $env:GITHUB_ENV -Encoding utf8 | ForEach-Object {
    if ($_ -match '^([A-Z][A-Z0-9_]*)=(.*)$') {
        [Environment]::SetEnvironmentVariable($matches[1], $matches[2], 'Process')
    }
}
```

- [ ] 保存 Cargo JSON、product-worker-receipt.json、protocol-driver-receipt.json、退出码夹具摘要。扫描程序必须来自固定 SHA 的真实构建，不从邻接清单自行建立信任。运行前确认 DISKGRAPH_SCAN_DRIVER_FIXTURE 已载入；缺少时 driver 测试会明确失败，不计为产品行为验收。
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

完整 workspace 与 CI 同样分成 foundation 和 entry，各自保留 1800 秒；默认测试并发及 all-targets/locked/no-fail-fast 不变。准备夹具后使用下方原生 Job 包装器，不再把两段合计限制为 1800 秒。WebCodex 外层执行可设置 4500 秒，给两段共 3600 秒以及准备、回收和回执留出余量；这是验证总流程的期限，不改变产品扫描或原打包负载期限。需要 Python 3.11 或更新的已有解释器。超时仍算失败；只有本段独占 Job 实际退休后才执行后段，回收不确定时拒绝继续。

```powershell
python scripts/run_windows_workspace_validation.py --output-dir "$dgEvidence/workspace"
cargo test -p diskgraph-store --lib --locked -- --test-threads=1
cargo clippy --workspace --all-targets --locked -- -D warnings
python -m unittest discover -s scripts/tests -v
python scripts/test_scan_worker_package.py -v
python scripts/test_worker_manifest_admission.py -v
python -m unittest discover -s scripts/tests -p test_windows_acceptance_job.py -v
cargo test -p diskgraph-engine --test content_cloud_windows --locked -- --nocapture --test-threads=1
```

- [ ] stable 与 1.97.0 分别记录完整测试结果；换 toolchain 使用 `cargo +<toolchain>` 并重新构建、绑定该组 worker，不能只修改版本标签。
- [ ] 分段包装器实际在 Windows 执行并保存 foundation.log、entry.log、receipt.json；本机契约测试不等于此项通过，任意阶段失败/超时/回收不确定仍保持完整验收未完成。
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

2026-10-09：提交 `027744a236a622388190569a1a60b8198aeaa108` 的 Windows 原生 CI 包任务 `113853075490` 已实际通过原 300 秒外层门禁。200000 文件、200001 节点、32 查询/4 客户端与 6/6 行为检查保持原值；造数据 25.516 秒、扫描 75.295 秒、清理 13.713 秒，查询 p50/p95 为 25.007/35.720 ms。正式 HTTP 检查 14/14（含实际 60 秒、2349 次请求、匿名/恶意 Origin/无数据库授权/实时撤权），stdio 检查 18 项通过。日志和原 artifact 报告保存于 `docs/benchmarks/ci_37940369957_windows_package_2026_10_09/`。这是 Windows CI runner 的包验收，不能代替下方台式机复验、后续提交或长期 soak；本轮 Windows 双 Rust 完整 workspace 在同一 SHA 的 CI 中已终态成功；不把其他平台的失败改写为通过。

2026-10-09 复验：提交 `5fd11d6c0b190f88f0f81c212281ae7693e3efa3` 的原生 CI 包任务 `113877279742` 再次通过原门禁。200000 文件/200001 节点、32 查询/4 客户端与 6/6 校验保持不变；扫描 48.552 秒，查询 p95 35.923 ms，HTTP 14/14。原始报告及日志以无损 gzip 保存在 `docs/benchmarks/ci_37947400346_windows_package_2026_10_09/`，receipt 绑定实际制品及原始字节摘要。不同 runner 观测不视为受控性能前后对比；本轮矩阵已终态 21/23 成功，Windows 双 Rust 完整任务均通过；两个 GNU 包兼容性检查失败。台式机已从相同 SHA 的独立 Git archive 开始原生分段测试，原 checkout 未修改，结果待验收。

- [ ] 正式验收沿用 accept-readonly-package.py 的 300 秒外层期限，覆盖造数据、扫描、查询和清理。文件数必须 200000、节点数 200001，32 查询/4 客户端及 6/6 行为校验不减少。
- [ ] 记录造数据、扫描、查询 p50/p95、清理、全流程、数据库/WAL 增量和硬件/后台负载。旧 d8d3995f 失败后诊断为 670.766 秒（252.347 + 164.821 + 249.023 等），不是干净配对基线。
- [ ] 如果正式流程失败，可另外运行 `python scripts/diagnose_windows_load.py --bin-dir target/x86_64-pc-windows-msvc/release --output-dir <新的独占目录>`。该脚本900秒诊断结果只能用于定位，不能替代300秒通过；clean_environment_confirmed=false 不改写为 true。
- [ ] 内存峰值和更长 soak 单独记录。60秒 smoke 不代表长期稳定；单台桌面通过不代表所有硬件或严格 RSS 上限。

## WD-07 回传与关闭规则

- [ ] 保存每项命令、环境、固定 SHA、二进制摘要、原始 stdout/stderr、退出码、测试数量、跳过原因以及全部 JSON/receipt；生成文件摘要清单。
- [ ] 逐项标记 passed / failed / not_run，报告首个业务失败与后续环境失败。缺权限、provider、toolchain 或原生部署材料均为未验收，不静默降级。
- [ ] 结束时核对源码未变化、没有未回收的本任务进程；失败保留目录供诊断，不擅自终止其他任务或删除用户文件。
- [ ] 将证据回传当前任务，只有精确对应当前源码的成功项才勾选。Windows 任务本身完成不等于全平台生产就绪；其他平台、监督恢复和归属采集仍独立验收。
