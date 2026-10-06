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


Windows 有限清理 API 开发 RED：CI 37393953210 / job 112045345026 / b4f5537 实际构建退出 101，7 个 E0432/E0599 均为 cleanup_progress/poll_cleanup 尚未实现。原始构建 stdout/stderr、失败 receipt 及摘要保存于 docs/benchmarks/windows_finite_cleanup_compile_red_2026_10_06；未执行原生行为测试，不计作行为验证。

macOS 产品验收新增精确分配字节断言：与默认上游 st_blocks*512 叶子聚合语义一致，错误已知尺寸和空 error 对象负测先失败后通过；Python 守卫 27 项通过。原 71c4eab 的排队 CI 保留，其较弱尺寸断言不作为本次新断言通过证据。


Windows 有限清理库层候选：冻结 480 份源码，archive SHA256 17199595a50ba8760413437a6c08c1a65f81041d0c7e70953864f1077b5c22e7；保留原30案并新增5案，等待实际 Windows 构建与行为结果。Pending/Err保留原owner，查询采用 Wait0/GetOverlappedResult(FALSE) 和同一绝对期限。旧cleanup/Drop、出生前pending、Registry有限恢复、目录删除和产品退出仍未完成，不启用产品路径、不勾平台父项。

当前本机产品全回归重复确认 CLI56/11/0、MCP119/34/0，原始日志保存于 docs/benchmarks/product_full_regression_current_2026_10_06，可信扫描宿主缺失导致的Unsupported未被跳过或改成查询成功；需要实际宿主接线后继续验收。JSON-RPC协议关联新增负测先失败后通过，Python守卫28项通过。


CLI完整原回归真实宿主接线：590份Mac候选源码 / archive SHA256 350dfb950d3be4c51e8d56ff378703005d712ad1bf19eda787f26b870a2cd74b。测试构造调用产品CliEngineHost，Recovery在临时目录前实际排空，缺host仍Unsupported；保持全部原授权/期限/预算断言，不导入种树、不跳过扫描。完整67案及原11失败案各实际ok为门禁，测试二进制前后hash绑定。独立静态复审CLEAR、本机no-run编译成功；29项Python守卫通过。受保护安装后的原生67案尚未执行，不能计CLI回归或产品完成，兼容Drop不宣称有限退出。


MCP真实宿主回归接线进行中：新增tests/service_fixture.rs，在真实部署材料存在时调用原open_with_scan_worker并在目录销毁前保留/排空同一Recovery；无部署材料时沿用旧服务，真实扫描仍Unsupported，不伪造revision。MCP lib no-run本机编译成功；tests/job_runner_fixture的实际闲置线程join/源目录生命周期用例1通过，不能代表活跃scanner或HTTP完成。独立复审发现原HTTP测试的detached accept/连接与drop(runner)不join问题，正在按physical-scan-process既有SSOT新增受控生命周期回归，不冻结/勾选完整MCP验收。


受控HTTP生命周期API开发RED已实际确认：cargo test --locked -p diskgraph-mcp --lib http_lifecycle_tests -- --nocapture退出101，仅E0432缺http_server_runtime；5个真实TCP测试尚未执行，不能计行为RED或通过。原日志、测试源码摘要及回执见docs/benchmarks/http_runtime_api_red_2026_10_06。实现需真实关闭heldsocket并join原accept/连接线程，不以连接计数为0替代结束。


HTTP受控生命周期本机最终验证：5个真实socket案与原连接上限1案均实际通过，原输出/最终源码摘要见docs/benchmarks/http_runtime_native_local_green_2026_10_06；不是Linux/Windows原生资格。10个原detached测试server改为owner，3个扫描源保活至runner实际join；停止不取消job，旧签名和原断言保留，fatal accept错误在全join后原样返回。Mac候选596份源码，完整CLI67+MCP159门禁冻结待原生安装环境执行。Python守卫30项通过，MCP全回归未计完成。


Windows有限清理库层原生GREEN：CI37395768727 / job112051232160 / 73fe226终态success；35项逐一核对原stdout均1passed/0failed/0ignored，含5项新增poll清理案。fixture SHA256 c7d33732e2d3507a1c73b4ca0840a089c14b95c8800135a8e7bce6a37a8c1cc7，完整原stderr/stdout/receipt及摘要保存于docs/benchmarks/windows_finite_cleanup_native_green_2026_10_06；首次归档因3项nocapture诊断位于测试名与ok之间而中止，保留诊断补齐归档，不修改原测试或输出。仅库层poll行为通过，未启用产品；旧Drop、出生前pending、有限Registry/Pool、目录删除和可信Windows扫描镜像/CLI/MCP资格仍开放，不关闭平台父项。


### Windows Registry deadline integration — native acceptance pending

Original Windows Recovery now exposes drain_until with the same absolute deadline through the original child poll. Pending, original errors and unwind return the unique owner to its original slot; capacity is released only after actual Complete. Taking work uses try_lock; returning the borrowed owner requires the original short state lock and does not claim a hard wall-clock bound. Pool/directory cleanup and product finite exit remain open. Four new tests are frozen with the existing 35 native cases (484 sources); Windows tests have not run on this macOS host. Qualifier guards: 25 passed; guard missing API RED is preserved, not presented as native behavioral RED.

Current-source engineering checks separately found strict product Clippy 44 errors; three concrete lint errors were fixed without suppressions, leaving 41 unused candidate-path errors. macOS load policy 8/8 and installation lease 6/6 regressions passed; pinned vendor 4/4 passed; package fmt check passed. Current macOS installed product run 37397304162 remains queued on ARM and Intel. Full production acceptance remains unproven. Raw proof: docs/benchmarks/registry_deadline_pre_native_2026_10_06/.


### Windows Registry original-owner native GREEN; directory missing-name RED/GREEN

CI37398546354/job112060229533 completed success on actual Windows, checkout f06b08e6f2f9fa2b8389dbdf2a1c4956fd9be4ea. All 39 named cases were verified against their original stdout: each 1 passed, 0 failed, 0 ignored; fixture SHA-256 60a1c35bef6175d1f8136a1471e4d8a28a267c86e8b985c957577c63a9837109. Original receipt and all 78 stdout/stderr logs are preserved in docs/benchmarks/windows_registry_deadline_native_green_2026_10_06/. This completes the Registry deadline sublayer only, not Pool, Prepared I/O or finite frontend exit.

A real macOS directory rename exposed cleanup treating the missing old name as Complete (1 failed/1 passed). Cleanup now rejects unconfirmed original-object deletion and retains original identity/allocation responsibility. A capacity-ledger fixture initially failed because macOS /var is a symlink (17 passed/1 failed); only the fixture parent was canonicalized, preserving the lease prohibition. Final private-directory regression: 18 passed/0 failed/0 ignored. Raw RED, intermediate failure and GREEN retained in docs/benchmarks/private_directory_missing_name_2026_10_06/. Qualifier guards 26/26 and explicit workspace-package fmt check passed. Frozen Windows candidate 485 sources/41 named cases now includes the two missing-name tests; native acceptance pending. This fail-closed fix does not prove handle-relative deletion or eliminate the existing path identity/delete race.


### Windows atomic private-root anchor — actual native controls pending

Frozen 488-source/45-case candidate adds atomic NtCreateFile FILE_CREATE under the original held parent and protected DACL, immediate external owner adoption before status/identity errors, retryable original-anchor confirmation after successful creation, and original FileID/volume checked ReOpenFile. Existing owner slots cannot be overwritten; failed native creation is never promoted. Four new actual Windows controls cover collision/invalid components, rename/foreign replacement, unmodified shareREAD lease with real managed Git cwd, and real SDK junction replacement with external sentinel. The native NtCreateFile/ReOpenFile combination is not assumed from static compatibility; all four must run. Guard missing-API RED and 27 guard GREEN are preserved; engine/MCP package fmt and locked metadata resolution passed on macOS. No Windows native execution yet. New raw proof: docs/benchmarks/windows_atomic_private_root_pre_native_2026_10_06/. This sublayer does not delete directories or close walker, Pool, Prepared I/O or final process-exit gates. Existing 41-case Windows run 37399209639 and macOS ARM/Intel product run 37397304162 remain queued at the last verified observation; no restart or cancellation was made.


### Prepared Connect ownership candidate — native result pending

Windows pipe preparation installs original server/event/OVERLAPPED/buffer into an external owner before ConnectNamedPipe. Prepared, Connecting, Ready and Reading are distinct; Connect cancellation does not create read EOF. Old synchronous creation remains a trusted compatibility wrapper. Full WindowsChild Job/process birth phases and product integration remain open.

Five cases require actual pending Connect: expiry, query failure, actual connection followed by pending Read, panic after submission, and cross-thread original-address preservation. Frozen candidate retains all previous 45 cases (50 total, 491 source files). Native Windows RED/GREEN has not executed for these five cases; macOS checks do not establish Windows compilation or acceptance.

Local native_child regression: 35 passed / 2 failed / 0 ignored. Failures: explicit_null_preserves_two_checkpoints_and_all_control_methods_are_unsupported; worker_control_checkpoint_failure_keeps_original_non_clone_error_and_real_reap (missing ready fixture). Both remain unresolved; no assertion weakened. The archive safeguard correctly rejected the obsolete 488-file count when three modules were added; expected inventory updated to 491, not relaxed to an inequality.


### Native results received after Prepared Connect dispatch

Run 37397304162, source 83dde527d0cee7a30e6f86a5e3c9bd450615d3bb: macOS ARM and Intel both passed. Downloaded original receipts and stdout prove on each platform CLI 67/67 and MCP 159/159, zero failed/ignored/filtered; ordinary UID 501 actual CLI→MCP→CLI observed a new published revision and 3→4 actual files. This is the 596-source feature-enabled candidate, not default deployment or full same-source platform acceptance. Raw evidence is preserved under docs/benchmarks/macos_products_native_green_2026_10_06.

Run 37400045249: 42/45 Windows cases passed. Three new root positives failed with original ERROR_INVALID_PARAMETER (87) at ReOpenFile, including moved-root, managed Git/lease and junction controls. No negative-only acceptance: root anchor delete reopening remains unqualified. Original raw failures are preserved under docs/benchmarks/windows_atomic_root_native_red_2026_10_06. New 50-case run 37401044695 is queued on b1787eade4bdd2bb17be79bbedfb219a3664cf28; its snapshot still contains this unresolved root defect.


### Windows original directory reopening fix candidate — awaits native execution

The original three positive failures (run 37400045249) are preserved. Replaced the unqualified NtCreateFile→ReOpenFile combination with OpenFileById: original held root is the volume hint, the existing allocation record supplies its full 128-bit File ID, and both original and returned handles are checked against that recorded volume/identity/type. No path fallback, truncated ID, privilege enabling, or lease-sharing relaxation. References: https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-reopenfile and https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-openfilebyid . The documented CreateFile prerequisite explains why the old combination was not guaranteed; it is not asserted as the sole proven cause of error 87.

Unmodified moved-root, junction/sentinel and managed-Git positive assertions remain. Added a fifth root case requiring the original shareREAD child lease to produce actual ERROR_SHARING_VIOLATION for DELETE reopening, then successful same-object reopen only after that lease is released. Candidate inventory: 491 source files / 51 exact cases, previous 50 retained. Mac formatting and Python mounting/inventory safeguards can pass without proving native Windows behavior; full directory walker, Pool, birth Job ownership and production launcher remain incomplete.


### Unix concurrent checkpoint diagnostic

Both previously failed Unix input cases pass individually (one actual passed, zero failed/ignored each); original logs and current-source hashes saved under docs/benchmarks/unix_birth_checkpoint_contention_2026_10_06. The concurrent 35/37 run remains failed. Current NativeBirthGate::acquire invokes the legacy checkpoint repeatedly on contention before real birth. This can advance lifecycle-count callbacks before their expected post-birth phase; deterministic contention regression and a separate admission/lifecycle contract are needed. Do not serialize the acceptance suite or remove original cancellation/cleanup assertions to hide this issue.


### Unix admission/lifecycle separation — local RED/GREEN verified

Actual deterministic contention RED: the second legacy lifecycle callback reported (2,false,false), i.e. before gate release and before the ready marker. Implemented separate admission callback for gate waiting; trusted compatibility spawn_with_input preserves two lifecycle phases and explicitly does not promise a gate-wait deadline. Product UnixProbeChild now calls spawn_checked, borrowing the same ProbeBudget for waiting and lifecycle checks. Worker compatibility session spawn also reuses its original check for admission. NativeBirthGate itself and native qualified launcher retain existing cancellation/handoff checks; original non-clone waiting error test routes through the explicit checked entry with original assertions unchanged.

Default parallel native_child regression: 39 passed / 0 failed / 0 ignored, including old two failures, deterministic legacy contention and distinct admission cancellation with only one lifecycle call/no child. Product probe regression: 19 passed / 0 failed / 0 ignored. Engine fmt passes; qualifier safeguards 29 pass. No forced --test-threads=1 for this subgroup. Logs and source hashes: docs/benchmarks/unix_birth_admission_fix_2026_10_06. The local native-child log predates only removal of one unused import; no behavior/assertions changed thereafter.

UnixChild birth impl extracted to unix_child_spawn.rs (153 lines), original object owner remains in unix_child.rs (388 lines); no type duplication or stub. Frozen Mac candidate now 597 sources; same root/helper/ordinary UID/actual product/CLI67/MCP159 gates plus actual default-parallel native child39. Still feature-enabled qualification, not deployment or full-platform production acceptance. Windows 51-case run 37401420174 remains queued; preserve prior 50-case run 37401044695.


### Windows prepared Job/I/O external owner — native RED/GREEN pending

Before first Connect, WindowsChild now installs original KillOnClose Job and Prepared phase in the caller external slot; stdout/stderr/control server operations are installed into that owner's optional pipe fields before submission. CreateProcess entry sets Creating; valid original process ownership sets ProcessOwned; native failure sets CreateFailed without taking the owner or setting cleaned. Missing process is acceptable only in confirmed Prepared/CreateFailed, and only with actual original Job-zero/I/O completion; Creating/ProcessOwned missing process and born missing stdout/stderr reject completion. Old synchronous preparation/cleanup/Drop remain explicit compatibility limits, not finite product birth scheduling.

Kept original Null four checkpoints, third non-clone error, postbirth wait/query failures, three inherited handles and normal whole-Job tests. Five new cases: real prepared Job+pending Connect capacity/expiry, original Job query error/retry, pending-Connect panic, unknown phase/missing leader state-fault rejection, actual CreateProcess invalid-cwd failure retaining original Job. The last case first verifies original native context and prints actual OS error; the baseline permits only its actual lost-owner assertion RED, not compilation failure, unrelated panic, or zero tests. Unknown-phase fixture is explicitly a state-fault test, not evidence of an actual unknown kernel result.

Frozen candidate: 495 source files, 56 exact cases; original comparison: 491 old source files plus the same native failure case. Original 51-case assertions retained. Qualifier safeguards 32 pass; Engine fmt passes; local macOS native_child 39 pass/zero failed/ignored. This local run does not compile or execute Windows paths. Records/source hashes are under docs/benchmarks/windows_prepared_owner_pre_native_2026_10_06. No Windows scanning platform capability enabled; all production acceptance parent items remain open.


### Windows native failures preserved; directory sharing and nested visibility corrections

Run 37401420174 (49b6d8d) actually executed 51 cases: 50 passed; the sole failure was original_share_read_lease_blocks_delete_reopen_until_released receiving Ok instead of ERROR_SHARING_VIOLATION. The four atomic-root identity/junction/real Git controls and five real pending-Connect cases passed. This does not approve the failed suite. Run 37403116203 (3aa73c4) confirmed actual old-source CreateProcess lost-owner RED, then candidate compilation failed with two E0624 errors: prepare_input visibility narrowed after moving its impl into a nested module. Original raw receipts/stdout/stderr compressed without rewriting under docs/benchmarks/windows_prepared_owner_native_red_2026_10_06 (113 files, original hashes).

The compatibility method now has explicit visibility confined to crate::native_child::windows. Original GitDirectoryLease root/component opens request FILE_LIST_DIRECTORY together with FILE_READ_ATTRIBUTES; the share-read flags, held parent-relative traversal, no-follow/no-recall flags and original failure assertion remain unchanged. Attribute-only access was insufficient in the observed native control. These changes require new actual native proof, especially permissions/ordinary Git compatibility; no ignored/skipped test replaces the control.

The checked Windows birth entry now separates original admission budget from four lifecycle callbacks; product probe borrows the same original ProbeBudget. Three additional prepared cancellation/actual pending deadline cases bring the frozen inventory to 59 with 495 source files, archive SHA256 22c9bd38c31ed55b082ac921f22d5a7423adcbf0459d1997d5cac807698aa8fa. Old-source RED verifier accepts native marker interleaving but still requires the exact one failed test, original lost-owner assertion and numeric actual OS error. Qualifier safeguards 33 pass; Engine fmt passes. Windows native execution for this source remains pending; finite product cleanup and trusted Windows scan deployment remain unfinished.


### Product expired cleanup boundary — actual old-source RED scheduled

Current Windows product execute still calls synchronous child.cleanup after collection; finite poll_cleanup is not yet routed here. Added expired_managed_probe_transfers_original_owner_without_legacy_wait: actual managed birth, original 10s budget exhausted inside postbirth hook, exact original deadline diagnostic, no consumption of legacy wait hook, same recovery capacity retained, then actual leader/Job-zero recovery after disarming. Test finally rescues original responsibilities on failure. The baseline adds only this test to its original 491 sources; production algorithm remains unchanged. Archive SHA256 86f060ea1d556f23c0aa3fc32d14f46f3d5c05265eb9c83121c9afb468d5b6d3. Driver requires exactly one failed case, actual consumed-boundary marker 1 and exact expired-legacy-wait assertion; birth/compile/unrelated errors do not count as RED.

Local qualifier guards 34 pass and Engine fmt passes. Native RED pending; implementation intentionally not claimed complete. Candidate 59-case source remains cead51b archive, without this new product deadline behavior. New comparison must prove RED before routing original deadline and finite cleanup into product. This is an additional production blocker, not evidence that overall platform acceptance passed.


### Current default worktree integrated RED — not hidden by feature candidate results

Actual cargo test --workspace returned 101: first CLI suite 55 passed / 12 failed / 0 ignored, all twelve scan-dependent preparations returned Business(Unsupported). Default macOS ScanWorkerRuntime explicitly refuses launch without the feature-enabled qualified installation path. Frozen feature-enabled CLI67/MCP159 proof does not imply this default worktree passes. The command stopped at this first failed suite; later suites are not claimed executed. Engine lib emitted 42 warnings. cargo fmt --all --check reported differences exclusively in immutable vendored disktree-core (12 paths); no vendor rewrite performed. Explicit fmt for all ten owned workspace packages passed (exit0). Raw logs and receipt preserved at docs/benchmarks/current_default_workspace_red_2026_10_06. Default installation/product integration and strict Clippy remain production blockers; do not remove tests or bypass trusted installation refusal to make them green.


### Full frozen Engine gate added; ordinary installed environment required

Current default worktree no-fail-fast execution reached Engine lib: 447 passed / 105 failed / 7 ignored; later targets still running. Default unsupported scans cause many preparations to fail; these results cannot be replaced by native-child39 or feature CLI67/MCP159. The installed macOS qualifier now inventories the same frozen Engine executable with --list and --ignored --list, requires unique/subset inventories, runs the complete binary with default parallel execution and no test filter, and checks exact total-minus-ignored passes, zero failures/measurements/filter. Minimum 559 total cases prevents a small subset from impersonating the full Engine. Ignored names are preserved in receipt and are not counted passed; required root and six ordinary cases still execute separately before this gate.

Driver guard actual RED (missing full regression validator) then GREEN; all qualifier safeguards35 pass. Logs: docs/benchmarks/macos_full_engine_gate_2026_10_06. Candidate source/binary installation policy is unchanged (597 frozen files). Gate runs after existing actual products/CLI67/MCP159, preserving their evidence even if broader Engine fails. Native full gate not yet executed; not full workspace/FFI/packaging/production proof. Existing queued jobs retain their original scripts and source; no queue restart used as progress.


### Complete default no-fail-fast run and two test contract corrections

Actual original no-fail-fast workspace handle30230 terminated exit101. 128 suite receipts sum to 1489 passed / 444 failed / 22 ignored; these are receipt sums, not unique vulnerabilities or a final corrected-source pass. The binaries compiled before the latest test corrections. Many failures originate in default macOS unsupported scan setup; FFI and other out-of-scope gates remain recorded rather than hidden. Original log is under docs/benchmarks/integrated_workspace_and_gate_red_2026_10_06.

Legacy missing-directory test conflicted with accepted fail-closed contract. Replaced success-on-missing assertion with real moved-original public complete rejection, original primary/cleanup diagnostics and retained payload; restored the exact original object and actually completed cleanup, including panic finally. Local exact case1/1 passes; production cleanup logic unchanged. Frozen macOS597 source overlays only this corrected test, preserving all product/native assertions.

Source layout walker ignored Rust path attributes and tried nonexistent nested paths. Added isolated declaring-file sibling plus normal nested module case: actual RED from missing file. Walker now resolves explicit relative paths from the declaring file directory, preserves ordinary module bases and inline-module rules, rejects absolute/escaping paths, and reports original missing source path. Routing case passes; full layout gate still fails with105 Chinese parameter/return documentation violations, now visible instead of aborting before inspection. Four of five layout cases pass; no gate disable or production cfg exclusion introduced. All layout/cleanup and original workspace logs preserved; full architecture conformance and production acceptance remain incomplete.

Final routing safeguards: four targeted gate tests passed, zero failed/ignored, two other full-source gates filtered for this targeted check; explicit ../ and absolute path rejection each verified before source reading. Full gate is still not passed. macOS597 corrected-test candidate archive SHA256 b19be8a7851a2812d3488a7699bb2832949f57c191503db5494d4484cfde37b0; production bodies unchanged. Qualifier safeguards35 pass, Engine fmt passes.


### Engine Chinese public callable contracts — current source layout GREEN

Resolved105 exact callable documentation failures across40 current Engine files. Existing parameter/return semantics preserved and normalized with explicit Chinese labels; missing entries receive actual ownership, original budget, return/error and platform limits rather than placeholder Java provenance. No non-doc source bytes changed across all40 files (before/after/non-doc hashes recorded). Original complete layout gate was RED with105 failures; actual current corrected Engine plus worker gate6/6 passed, zero failed/ignored/filtered. Default library42 warnings and production installation/default scans remain open.

Reproducible documentation.patch.gz plus raw GREEN and SHA manifest: docs/benchmarks/engine_doc_contract_2026_10_06. Frozen overlays accept only exact non-doc equality: macOS597 archive applies16 files, defers24 whose bodies differ or files are absent; Windows495 archive applies36, defers4. No unverified current body replaced a frozen body. Therefore local current-source6/6 does not claim either frozen archive's entire layout gate passed, nor default workspace444 failures fixed. Candidate SHA256 macOS25bbea7579e67e13f5ad89b1a27a070fc0d21a1d1251de5ac808f0f3beb26d50, Windows013b0085c6a52d9a6b036b979ced5512d08b68662a0e3396cf766e77d903dbc2. Qualifier safeguards35 pass; Engine fmt passes. No native behavior rerun is claimed for these comment-only overlays. The broad source integration and same-source full production gates remain incomplete.


### Actual expired product cleanup RED; finite probe candidate and MCP layout GREEN

37404115058 and37404310871 both terminal FAILURE: each actual Windows candidate56/59 passed; unchanged three failures are moved-root and moved-junction rename ERROR_SHARING_VIOLATION32 after increasing lease read access, plus held child share-read DELETE reopen unexpectedly Ok. The latter means the earlier permission addition did not close the target control. Retain all three failures and assertions; no native sharing cause/solution inferred as proven. Whole platform remains unaccepted. Raw originals under docs/benchmarks/windows_probe_expiry_native_red_2026_10_06.

37404310871 original491-source baseline actually executed expired_managed_probe_transfers_original_owner_without_legacy_wait: one failed, zero passed/ignored, native marker DG_EXPIRED_PROBE_BOUNDARY=1, exact expired-legacy-wait assertion, actual duration11.01s; verifier verified_target_red. With RED established, product Windows execute now calls one nonblocking poll_cleanup on the same ProbeBudget absolute deadline. Pending/Err transfers the same child to the original reserved slot; no legacy cleanup or Drop retry. Successful collection also rechecks original budget, preserving Deadline/Cancelled as primary and cleanup as secondary. Unknown/pending responsibility is never Complete; compatibility shutdown drain, Pool/directory work and OS single-call limits remain open.

Windows495-source candidate now60 exact cases, including the same expired product test; SHA256 c02549e9d3c810c60aa24b57df3c4343c9089a3e0ffba9c07968428f6032e9b0. Native GREEN for new product boundary not yet run. Old-source RED snapshot remains unchanged; directory controls have not been weakened.

MCP layout failure corrected without expanding original six public module paths: new HTTP owner module is private with explicit root type export. Preserved all five actual HTTP lifecycle tests (5/5). Moved runner fixture inline test into real tests/job_runner_fixture/tests.rs, preserving qualified test name/body and actual idle owner test1/1. Layout now inspects main entry modules too, resolves main's directory base correctly, and checks actual Windows key ACL callable documentation. Full MCP source gate9/9 passes, Engine/worker6/6 passes. macOS candidate now598 sources, SHA256369c5e64f0ba1025104cef0ce45bd8be3585ff30aabe6de00f89a47f163664ad; CLI67/MCP159/full Engine native gate still required. Logs at docs/benchmarks/mcp_source_gate_and_probe_cleanup_candidate_2026_10_06. Qualifier guards35 and owned Engine/MCP fmt pass. This is not a default integration/workspace/Clippy/platform readiness pass.


### Windows same-object granted-access diagnostic — no sharing assertion relaxation

60-case production cleanup candidate37406293798 and macOS598 full Engine candidate37406297508 revalidated queued. Directory controls still have three actual native failures; no production root or lease algorithm changed in this diagnostic. The held-child control now verifies original anchor/leaf full identity before asserting native sharing failure, and queries actual original anchor, child lease and unexpectedly successful DELETE reopen via SDK NtQueryObject/ObjectBasicInformation/PUBLIC_OBJECT_BASIC_INFORMATION. Only success status prints granted mask/count; nonzero status is preserved without interpreting zero-initialized data. No path reopen, deletion, permission enablement, additional owner or background thread. Existing expected ERROR_SHARING_VIOLATION and both rename controls remain unchanged.

Added only windows-sys0.61.2 WindowsProgramming feature to use SDK ABI; no version/vendor pin/source change. Frozen Windows495-source/60-case diagnostic archive SHA256 bfca6956a161f20ba8f280b8a6a6922382a890321f5c61ff6a0fa7bc8aad2a50. Local current-source layout6/6, qualifier guards35 and Engine fmt pass; this does not compile/run Windows or identify the sharing cause. Local log: docs/benchmarks/windows_root_rights_diagnostic_pre_native_2026_10_06. Directory bug and all production parent gates remain open; actual diagnostic required before choosing further lease/root behavior changes.


### Original registry owner survives cleanup unwind

Three bound native runs37406780904/37406293798/37406297508 revalidated queued; none restarted or claimed passed. Actual current compatibility registry drain had no unwind guard after moving Retained to Draining: an injected panic before original native cleanup dropped the live original child and permanently orphaned its slot. Real macOS /bin/sleep child regression actually RED0/1 with the exact lost-original-owner assertion. Fixed the actual drain path by catch_unwind around original cleanup, returning the same owner to the same slot before resume_unwind with unchanged payload. Ordinary cleanup errors likewise restore owner before error projection; completed owner destruction and projection now occur outside the registry lock. No new wait deadline, hidden thread, duplicated child or false Complete.

Actual local registry6/6 includes unwind and real error/retry controls; native-child39/39, Engine/worker source layout6/6, owned Engine fmt and OpenSpec strict pass. Qualifier guards35 pass. Raw RED/GREEN/regression/layout/guard logs and reproducible patch/test source under docs/benchmarks/registry_cleanup_unwind_2026_10_06. Linux/Windows execution of this changed compatibility path not performed. The finite shutdown/Pool/default integration blockers remain open.

macOS frozen source overlay changes only its exact original drain body and mounts the new actual test source (599 total); archive SHA25629d8dd3165e6663ee1266c7f0bbdb36621c2d9c954679529b000c295569a4855. Current Windows-only registry modules are not injected into this Mac snapshot. Existing actual product, CLI67/MCP159 and complete Engine gates remain unchanged. Windows diagnostic snapshot495/60 unchanged and still queued. Frozen source qualification is not integrated default production code; broad worktree integration and same-source complete workspace/Clippy/platform/performance gates remain required.


### Coherent product source integration checkpoint — not production acceptance

Integrated the actual234 modified/new source and build files of Engine, CLI, MCP and scan worker plus the lockfile, including original-owner unwind restoration and Windows original-deadline probe cleanup. Selection is explicit by owned source/build scope; README/architecture/user edits, unrelated benchmark materials and other dirty files are preserved outside this commit. Previously these implementations primarily existed in the worktree/frozen source qualifiers; this checkpoint makes them actual checked-in product sources. No branch switch, vendor modification, release or platform capability bypass. macOS candidate feature and Windows fail-closed scan launch remain in place.

Current all-target workspace check passed; Engine/worker source gate6/6, MCP source gate9/9, native-child39/39 and owned four-crate fmt passed. Strict Clippy actually RED: initial46 production and11 test errors (some overlapping diagnostics) included doc blank lines, test hook complexity and redundant boxed String. Fixed those documentation/test issues without lint suppression; current strict Clippy still fails42 default Engine unused-import/dead-code diagnostics. Non-Clone original error control now checks the same boxed byte payload address and original bytes, retaining unique allocation identity. Test-only raw FD hook is a real separately mounted cfg(test) responsibility module; the structural gate originally rejected an inline alias and then passed6/6 after the actual split.

Complete default workspace regression is not accepted (previous complete run recorded444 failures; not counted as current rerun). New same-source full platform/strict-Clippy/default scan installation/shutdown/performance gates remain required. Earlier frozen native pass reports are not substituted for this larger integrated source. Current file SHA manifest and original local logs at docs/benchmarks/product_source_integration_2026_10_06. No production task checkbox added. Bound macOS599/Windows495 native runs remain queued and are not restarted solely because of delay.

Integration inventory correction: initial porcelain status collapsed untracked directories and omitted five nested test/fixture sources (CLI test_engine/tests, Windows exit C fixture, Engine flow fixture, MCP runner test). Expanded authoritative git ls-files --others inventory identifies and integrates them; original234 manifest now239 sources/build files. 73a8f81 alone is incomplete and not an accepted build. A Git-exported source compile is required before claiming a coherent checked-in build. This correction does not alter test assertions or platform capability gating.
