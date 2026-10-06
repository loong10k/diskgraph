# 2026-10-06 桌面只读原生验收记录

规格事实源仍为本变更的 physical-scan-process.md、specs 与 tasks。该记录不关闭任何父任务，也不声明发布版本或全平台验收完成。

| 源码 / 实际运行 | 观察结果 | 结论边界 |
| --- | --- | --- |
| d957e66 / Windows CI37384689462 | 编译通过，四项跨线程 pending I/O 通过，三项清理恢复 RED | 真实原 owner 提前丢失与槽位释放问题已复现 |
| 436ebea / Windows CI37385193355 | 原七项全部实际通过 | 清理失败保留原句柄与真实重试已验证；出生与清理期限仍开放 |
| 7f2b5e8 / Windows CI37385724802 | 原七项继续通过，新增三项出生后错误/panic owner RED | 原错误/panic 保留，catch 外 owner 槽为空；待外槽出生实现 |
| 8878912 / macOS CI37381931634 | 临时卷 mkdir 非法 UTF-8 返回 EILSEQ，未进入扫描 | 不能作为 helper 扫描失败；必须修正实际文件名夹具 |
| 66c445a / macOS CI37385115407 Intel 产物 | 协议、root 安装、helper 正常/取消/panic、Engine 发布具备实际 PASS 标记；响应预算案未出生 | 原 32 字节夹具不能通过 Hello/预算终态准入，不是出生后溢出证据；记录产物身份，不替代尚未终态的任务状态 |
| 当前 macOS budget focused test | 显式候选 feature 下精确 1/0/0；512 字节真实准入，实际树编码 992，Hello+预算终态观察 230 | 仅本机原 scanner/codec；实际 helper 与 Engine 拒绝发布须由新 CI37386707636 验证 |

原始 stdout/stderr、receipt、candidate 源码摘要与实际测试程序摘要位于 docs/benchmarks/windows_cleanup_native_red_2026_10_06、windows_cleanup_native_green_2026_10_06、windows_birth_owner_native_red_2026_10_06、macos_installed_invalid_filename_failure_2026_10_06 和 macos_engine_budget_admission_failure_2026_10_06。

仍须完成 Windows 出生前外部 owner、探针的持久恢复域、有限协作清理、受信镜像与完整 helper/Engine/CLI/MCP 正向链路；Linux 旧 Conflict 首因、三平台内容/provider、当前全 workspace 与同 SHA 性能/原生门禁仍开放。资格快照不能代替当前未提交集成源码的完整验收。

CI37386707636 的实际 job112021625493 labels 为 macos-15-intel，回执 runtime architecture=x86_64，源码 c0ea6ca。两个协议/预算条件、一个 root 安装更新恢复、六个普通 helper/Engine 案均精确 PASS（9/9）；原错误/panic 文本属于通过案的预期取证，不单独算失败。原回执及九案日志保存于 docs/benchmarks/macos_installed_engine_native_green_2026_10_06。ARM job112021625944 待完成；此结果不关闭 CLI/MCP 正向、并发更新/发布或完整全平台门禁。

## Windows managed probe RED preparation

The fixed candidate now includes three exact production-probe regression cases: cancellation with native wait failure, cancellation with Job query failure, and the original String panic with native wait failure. They bind a real fixed-capacity ProbeHost but deliberately retain the original probe execution algorithm. Actual birth is independently witnessed; acceptance requires the original error or panic, retained owner/capacity, rejection of another reservation, and native wait/Job-zero observations before releasing the slot. No missing API, zero-test result, or compile failure counts as behavioral RED.

The prior 17 native cases remain unchanged. Local qualifier checks passed 7/7 and archive safeguards 8/8; native Windows execution remains pending. The frozen 466-file archive SHA-256 is `d35a44e6f5eef656365ac049b0bc233eccbe1ca335cf82121f18c3afcbea13a0`. These tests do not qualify the complete product.

GREEN must also transfer the corresponding Git private-directory owner with the failed child into the same bounded session recovery responsibility. Directory completion or Drop must not remove a directory still used by a retained child, nor retain unrelated sessions based on a global occupied count. Parent task 15.13 and full-platform readiness remain open.

## Latest native comparison and ARM64 completion

macOS run 37386707636 is now terminal success on both architectures. ARM64 job 112021625944 actually passed all nine cases, each with an exact 1 passed / 0 failed / 0 ignored result; its ordinary UID, source SHA, helper and fixture hashes are retained in `docs/benchmarks/macos_installed_engine_native_green_2026_10_06/arm64/`. This does not close positive CLI/MCP, update/publication concurrency or complete integration.

Windows same-runner run 37388084630 completed: current frozen source passed 14/17, original frozen source passed 11/17. Both failed the same three unchanged normal-exit exact-count assertions (2 versus 1); only the original additionally failed the three birth-owner cases. The original receipts and all logs are retained under `docs/benchmarks/windows_same_runner_baseline_comparison_2026_10_06/`. Before changing any assertion, observe the actual original Job member IDs and native process identities. Parent tasks remain open. Managed-probe RED run 37389541495 is pending.

## Windows member identity diagnostics

The next frozen candidate adds only cfg(test) read-only Job member diagnostics to the normal-exit fixture: at most four nonzero observations per fixture and 64 PIDs per observation, without retries. It logs the original held leader PID, actual Job PID list, queried-handle Job membership, native creation/exit FILETIME and image path, with original error codes and elapsed observation time. Query handles close immediately; no child birth, termination, original assertion or production normal-exit algorithm changes. Enumeration and process-handle observations are explicitly non-atomic and do not prove normal exit. The 467-source archive SHA-256 is `316c1e7377a5767d7c9af791dda372b06ff0f1f1ec661b2fa327f6b05a2d7c79`; qualifier 7/7 and archive checks 8/8 passed locally. Native execution remains pending.

## Actual managed-probe behavioral RED

Windows run 37389541495 / job 112031070977 successfully built and executed 20 fixed cases. All three managed-probe cases witnessed actual birth, verified the original cancellation or String panic and one native cleanup fault, then failed at retained occupancy: 0 instead of 1. The original 10 I/O/cleanup/birth cases passed; the same three normal-exit count failures remain separate. Raw logs and receipt are preserved in `docs/benchmarks/windows_managed_probe_native_red_2026_10_06/`. The worktree GREEN now reserves before birth, holds both owner states outside catch, performs one explicit cleanup and retains the original native owner before error projection or original panic resumption. This has not yet passed Windows native GREEN or directory lifetime acceptance. Local existing probe regression 19/19 and Git cleanup 5/5 passed; feature-enabled response fixture 1/1 passed. Full production acceptance remains false.

## Private directory lifecycle RED preparation

The fixed 469-source candidate retains the original probe and GitPrivateDirectory production algorithms. It adds a mandatory real native directory creation/write/identity/normal-completion prerequisite, followed by cancellation/complete and panic/Drop recovery cases with actual child birth and one native cleanup fault. The prerequisite must actually pass before either recovery failure can qualify as behavioral RED. Original-directory identity must remain until the original child is actually waited and its Job observed empty; only then may Recovery remove the directory and release capacity. Finally-only rescue never counts as production success. Archive SHA-256: `75ed63a9b0d2fd387f175c767aaed54dceb8ed2e9fa73fa6a8790e6a485e6fc8`. Local qualifier 8/8 and archive safeguards 8/8 passed; native acceptance remains pending. The worktree child-owner GREEN is deliberately excluded from this RED archive.

## Actual Job member observations

Windows run 37389867055 / job 112032135904 built and executed all 20 cases. In each of the three normal-exit exact-count failures, bounded native observations show the expected live leader or ordinary fixture descendant plus a real `C:\Windows\System32\conhost.exe` process. The queried process handle is associated with the original Job, with native creation/exit FILETIME recorded. The observations are non-atomic and later query failures retain their original errno. Frozen and worktree production creation flags already include CREATE_NO_WINDOW; no production flag or normal-exit algorithm was changed. Raw receipt/logs are retained in `docs/benchmarks/windows_job_member_native_diagnostics_2026_10_06/`.

The next test correction must identify and wait the expected ordinary process by its original handle/identity and Job membership rather than assert an unsupported exact total of one or filter OS member names. Nonzero Job accounting while that process is live, Pending before actual exit, original leader code, and complete original Job zero after natural release remain mandatory. Unknown extra members cannot be ignored. This correction and native verification remain pending.

## Probe owner GREEN and bound ordinary process candidate

The 471-source candidate integrates the independently reviewed catch-external Windows probe owner fix. It also corrects the normal-exit fixture to bind the expected ordinary descendant using the original std::Child handle PID/creation record and a held query/synchronize handle: exact identity, original Job membership and WAIT_TIMEOUT before/after Pending are required; actual signal/exit0 and unfiltered original Job zero are required after natural release. Live-leader cases verify their existing held original handle and creation identity. No OS member-name filtering, production normal-exit change or new creation flag is introduced. Local existing probe 19/19, Git cleanup 5/5 and enabled response fixture 1/1 passed; native GREEN remains pending. Private directory production code is deliberately still original, with its mandatory native prerequisite and two recovery cases retained. Only owner/normal-process component acceptance may pass; full production acceptance remains false.


### Windows 原目录 owner 真实 RED 与共同资源恢复候选

- 原生 run 37390619940 / job 112034553691 / source 4621b3ef70aca3e0ab384671cebfd38df493d0df：实际编译通过并执行 23 案。真实 Git 私有目录创建、写入及正常完成的前置案通过；取消 cleanup 失败和 panic cleanup 失败两案均在原目录已被提前删除处失败。不是 Unsupported、编译失败或未出生进程；原始回执与所有日志见 `docs/benchmarks/windows_private_directory_native_red_2026_10_06/`。
- 修复候选以固定 session 槽共同保留原 child 和目录 owner，恢复锁外先确认原 child wait/Job0，再按原目录身份删除。活跃 session、借出目录或删除失败不退还容量。新增真实 deny-delete 及旧代次保护两案；原 23 案断言不变。
- 当前冻结候选加入 Engine 注入入口，CLI/MCP 工作树接线保留外部 Recovery；该冻结仅覆盖 Engine，不能作为 CLI/MCP 原生验收。无限 wait/I/O、目录身份检查至删除的同权限竞态及全部生产父门禁仍未完成，不勾选 15.13。

- 前一 owner-GREEN 候选 run 37391313757 / job 112036795511 / source 963bcaa0f48dd2012b9a1f318c93fc0d97ace6c5 已终态：23 案实际执行，21 通过、2 失败。三个 managed probe 原 owner 恢复案及七个 normal/control 回归全部通过，每案真实 1 passed/0 failed/0 ignored。失败仍为原目录 complete/Drop 的两案；本候选尚未包含共同资源池，因此整体 CI failure 保留。全部原始证据见 `docs/benchmarks/windows_probe_owner_native_green_2026_10_06/`，不能宣称 Windows 产品就绪。
