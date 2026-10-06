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


### 产品 runner 生命周期复审与原 panic 回归

- 只读复审发现：探针资源恢复仅在退出触发会造成一次可恢复故障后持续占满容量；JobRunner::stop 原先忽略 worker.join 的 Err，吞原 String panic。新本机回归 `runner::tests::stop_preserves_original_background_panic_payload` 在旧实现真实失败（原后台线程确实 panic，stop 返回正常），修复后实际 1 passed/0 failed/0 ignored。
- 工作树增加 stop_and_join 保留原线程结果，旧 stop 保持签名且恢复原 panic；MCP join 后先处理原资源，再继续协议或后台原异常。Windows runner 使用宿主同一 Arc<ProbeRecovery> 在每轮新认领前单次 drain，未恢复时暂停认领，不重建 Host/容量、不把容量占用错误结算成业务失败；不创建独立隐藏 reaper。
- 本机 Git cleanup 5/5、CLI 原 panic 边界 1/1 通过，MCP 默认及 macOS candidate-feature 二进制检查通过，但存在原有未使用候选模块警告，不称严格 Clippy 通过。最新共同恢复 CI 37392833173 绑定 6420d11715eebdc0fc5d30f4cfeb741a40943bc3，实际结果尚待确认；此冻结不包含随后 runner 工作树修改。
- 无界最终 shutdown、底层无限等待/I/O 及实际 CLI/MCP Windows 产品原生行为仍开放。保持 Recovery 责任只是正确性前置，不能作为有限退出期限已实现的证明。

- 复审确认当前单 runner MCP 路径的运行期恢复/原 panic 接线；进一步增加 Engine 与 ProbeRecovery 的原池指针一致性校验，错配在启动 runner 前 InvalidArgument 拒绝。多 runner/并发 tick 的检查至认领容量竞争仍未验收，不扩展单 runner 结论。


### Windows 共同资源恢复 25 案原生通过及 runner 准入候选

- run 37392833173 / job 112041720352 / source 6420d11715eebdc0fc5d30f4cfeb741a40943bc3 终态 SUCCESS。原始回执及全部日志已核对：25 案均实际 1 passed/0 failed/0 ignored，原 23 案未删改，真实目录 deny-delete 和借出目录/旧代次保护两案也实际通过。证据：`docs/benchmarks/windows_shared_resource_native_green_2026_10_06/`。仅证明冻结 Engine child/目录责任恢复，不证明产品或有限 shutdown。
- 新 runner 候选保留上述25案，新增原后台panic、错配Recovery拒绝、真实线程忙时不认领、活跃session不误判任务失败四案。Windows managed 同Engine调度使用独立 try_lock 准入，持资格后复核原池容量；忙时保持Queued，查询不走此锁，跨进程继续原DB fencing。此锁不覆盖可信直接run_job路径，不宣称所有库并发入口均串行。
- 最终退出必须区分“有限业务返回并移交原Recovery”与“OS进程在固定墙钟内真正退出”。现模型中pending OVERLAPPED内存属于原进程，不能只复制句柄、Drop/forget或无限循环伪装有限退出。非阻塞cleanup、出生前pending prepared owner、原deadline及上层显式恢复责任仍待实施/验收；不勾选生产父项。

- 非作者复审发现前一候选执行后 StaleOwner/Conflict 的 continue 可能带入刚移交的owner；现每个候选认领前持同一资格复核原池，后续Queued保持不变。准入Mutex仅保护互斥资格，无业务数据，panic后可取回原guard并重新做容量检查；新增实际线程panic/原payload/资格恢复案。候选共30案；此新增测试只证明准入资格恢复，不代替完整任务重新执行或产品原生验收。首候选实际probe后fencing失效+保留owner+下一候选这一组合仍缺原生端到端证据。


### 非阻塞清理新 API 的开发 RED 冻结

新增三个实际 Windows 夹具：原 pending read/write 在已过期限保留原地址，移交到另一线程在同一原期限内实际取消和观察 ERROR_OPERATION_ABORTED；原 child 过期后保留原 Job/process，后续同 owner 真实 wait/Job0 才 Complete。当前仅测试引用新 CleanupProgress/poll_cleanup，生产入口尚缺；下一冻结的预期失败是 Windows 编译缺 API，只算开发 RED，不算已出生进程或行为 RED，也不算原30案回归失败。30案 runner 候选37393613831保持原源码运行，不取消、不据排队重新启动。新API编译/原生完成之前不修改旧产品清理/Drop路径；prepared owner、有限Registry/Pool、上层shutdown仍开放。


### macOS 实际 CLI/MCP 正向产品验收准备

沿用现有九案普通UID/root安装资格及同一受保护epoch2安装，在隔离数据目录新增真实CLI init --index-only、CLI node、MCP stdio initialize/node/index/status/node，以及正常EOF退出。所有扫描来自实际产品进程；不直接写store、不伪种树、不调用本机sudo或安装。验收绑定helper/Engine fixture/CLI/MCP各自实际二进制摘要及完整源码，必须观察完成job与包含其job ID的原revision，CLI/MCP对同一已发布节点的计数、server/scope/revision一致；未知或错误envelope、未完成job、非零退出均失败。两架构分别实际运行后才能称产品正向通过。

产品源码扩展仅通过显式allow_products挂载开关启用，普通Windows/旧挂载默认仍拒绝CLI/MCP范围；不得用宽泛任意路径白名单替代冻结源摘要/预算/拒绝链接校验。候选feature只供已明确选择的native资格，不提前声称默认发布构建已启用。完整回归45个旧失败及升级并发仍须单独收口。

- Windows runner 37393613831 / job112044235418 / source5649c9ad0cbaa4418f6e28cf714805f1bb8dc972 已实际30/30、0失败/忽略，原25案及新增5个runner责任/准入/panic案逐一核对原始1 passed记录。全部证据见 docs/benchmarks/windows_runner_native_green_2026_10_06/；不以此关闭有限cleanup、实际probe后StaleOwner组合或Windows产品门禁。
- macOS产品589源候选已准备：原九案仍保留，挂载产品包须显式allow_products；本机实际CLI/MCP候选feature构建通过但23条未使用候选模块warning仍在，不称Clippy通过。挂载/响应拒绝脚本23项通过（纯脚本检查不代替产品进程），下一步同源码ARM64/Intel原生运行，不在本机安装helper。
