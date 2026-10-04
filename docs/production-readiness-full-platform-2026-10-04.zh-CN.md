# 全平台验收续篇 — 2026-10-04

本文延续[全平台实施记录](production-readiness-full-platform-2026-10-02.zh-CN.md)，完整平台目标仍未完成。

## Windows 完整原生属性与暂存末检（D31前置，2026-10-04）

schema12 用独立80字节版本记录保存完整128位文件ID、u64卷序号、EOF和原始创建/写入/变更时间。
保留根链约束属性相对打开，每项内核查询前后检查原期限/取消；实时授权与fence最多复用20ms，
批次/发布强制复验。采样不占数据库写锁，末组件只观测reparse对象本身，旧content策略未放宽。
旧行保持未捕获；明确不一致记gap，旧allocated/dedup/目录尺寸或未知身份保持Unverified。

独立审查复现“等待图库锁逾期后仍提交2行staging，随后cleanup隐藏写入”，现暂存/发布fence回调
在等待锁之后末检原时钟及本机取消。回归保留独立写入事件，RED0/1、GREEN1/0覆盖超时与取消。
[验收记录](benchmarks/windows_observation_acceptance_2026_10_04.json)保留原始日志：Core9/9、
Store观测过滤10/10、Engine观测过滤24/24（包含既有用例），最终workspace **1089/0/18（45 suites）**，
严格Clippy/fmt/OpenSpec/release、14vendor摘要、stdio18/18、HTTP/SSE13/13、执行UniFFI19/19通过。
非作者代码APPROVE/架构CLEAR，最终44份源码摘要匹配；Windows新增10个Native和1个Engine
流水线用例须本批同源码CI执行，本机不能替代。

release观测主键窄读在20k/200k未选记录下均18条VM；macOS真实文件负载各4/4，扫描
**0.437/4.391秒**，top/children p50/p95 **6.469/8.564ms**和 **6.773/7.999ms**，数据库+WAL
**43,442,176/436,916,224字节**，直接CLI子进程最大RSS观察 **44,826,624/263,307,264字节**。
相对D30字段增加约2.1%存储；独立观测不证明配对提速、严格RSS或Windows原生规模扫描成本。

上游walk仍按路径；本批根句柄只约束补充采样。真实ReFS高位ID、provider不下载、旧scope编码
来源、可信历史身份利用、原生写、授权采集任务、宿主/mobile和签名部署仍开放；不新增完成勾选。


## 根名称重新绑定与 FFI 源码边界（D32，2026-10-04）

D31 源码 `45d732c9` 原生 CI 终态 20/22：两个 Windows Rust 任务证明属性句柄不必然阻止目录重命名。先补测试的 `5963dd0a` 也为 20/22：注册根被重命名并重新绑定后，旧实现仍返回 `Ok(())`。D31 记录保留两轮失败与原生日志。后续修复从保留父句柄核对当前 drive 和各原名称，比较完整卷/128位ID/创建时间及安全目录状态，允许目录修改时间变化。有限协作复核仍有检查后的竞态窗口，每次复核的原生工作量随根深度增长；新原生 CI 和 Windows 规模成本尚待验证。

FFI 按 API、realm、扫描、JobHandle、共享 JobState 和别名拆分真实实现。两份固定 include 保留旧 UniFFI 词法根；普通 mod 提取初版曾导致19项校验全部变化，修正后真实执行恢复19/19，没有覆盖checksum或包装函数。中文契约使用精确 Rustdoc 条件 `cfg_attr(doc, doc = literal)`。生产 AST 门禁解析真实包含源，并拒绝其他位置的 include；两个遗漏负例实际 RED 后 GREEN，门禁8/8。最终本机workspace **1097/0/18（46 suites）**、严格Clippy、限定fmt和OpenSpec strict通过。同源码原生验收仍待完成，全平台父项不勾选。

release Swift/Kotlin 绑定已重新生成；真实 Swift 宿主编译运行通过会话、分页、轮询、v1及系统 SQLite 共存调用，4份生成Rustdoc页面保留中文参数/返回契约。[D32验收记录](benchmarks/ffi_structure_root_binding_acceptance_2026_10_04.json)保存真实RED/GREEN日志、执行19/19校验及最终源码清单。Kotlin原生运行与Windows行为仍等待新的同源码CI。

首轮D32原生运行 `ca5292b` 的两个Windows Rust构建因测试专用 `path_digest` 重导出未使用而被 `-D warnings` 拒绝，根回归尚未执行。后续仅将该导入条件与唯一Unix测试调用方对齐，保持严格警告及生产行为不变；全部受影响FFI目标54/0、FFI Clippy及全目标workspace构建通过。上述1097/0/18属于前一完整运行，仍须新的Windows原生执行证据。

同一 `ca5292b` 的Windows release包实际通过20k文件原生负载4/4：扫描 **15.738秒**、查询p50/p95 **27.290/42.900ms**、数据库+WAL **54,067,200字节**。此证据覆盖修复后的真实扫描，不替代尚未执行的根单元回归，也不证明配对性能改善；Windows200k和峰值RSS仍未测量。


后续同源码 `c8ff78174e7d7b3f23e8410afc3d16595a48188c` 的 [CI全部22项通过](https://github.com/loong10k/diskgraph/actions/runs/37155851104)。两个Windows Rust版本实际通过全部13个选定的原生观测、根绑定及发布用例，含根与祖先重命名绑定；回执保存终态与Windows stable/MSRV、Linux ARM、macOS Intel日志。Swift/GRDB与Kotlin原生宿主作业也通过。本次完成D32原生验收；命名空间竞态、Windows200k/RSS、provider、原生写入、GUI/移动/真机/签名/生产环境门禁仍未完成。


## 历史尺寸资格与源码组织（D33）

真实旧公共API对未知/读取失败节点及类型替换返回数值增长，未知目录提前Same并暴露占位尺寸。`c8ff781`隔离旧源的正确回归为Core **3/8**、Engine **3/6**、FFI **2/3**（通过/失败）；最初误用根目录的夹具错误另存。Core现统一判断尺寸观察与准确类型，Engine比较每侧独立保留null，只统计有效Size/Contents尺寸变化；已知零值和负增长保留。

查询/比较对象分为17份真实文件，保留公开导出及序列化合同；独立复审确认46份旧函数体中43份未变，仅growth、changes、compare_entry改变。三项AST门禁覆盖本批源码子树并拒绝生产直接path覆盖，不代表整个crate或宏展开认证。

最终本机workspace **1125通过/0失败/18 ignored，48 suites**；新增回归 **11/9/5**、结构门禁 **3/3**、vendor **124/0/2**、14份上游摘要未变。严格Clippy、定向fmt、include显式fmt、构建与release通过；实际UniFFI **19/19**、stdio **18/18**、HTTP/SSE **13/13**通过。真实文件替换为目录的CLI/MCP对照 **3/3**，changes数据一致。非作者代码APPROVE、架构CLEAR，32源清单哈希一致。

| macOS release夹具 | 扫描秒数 | 查询p50/p95毫秒 | 数据库+WAL字节 | 单个CLI子进程最大RSS字节 |
| --- | ---: | ---: | ---: | ---: |
| 20k文件 | 0.673 | 11.970 / 18.079 | 43,442,176 | 44,990,464 |
| 200k文件 | 4.371 | 8.536 / 10.459 | 436,916,224 | 263,421,952 |

各夹具4/4，32次查询、4客户端。时间含CLI启动；RSS由macOS time逐个实际CLI进程测量，不是并发总RSS。非配对观察不能证明提速、严格RSS或历史查询吞吐；Windows200k/RSS仍未测。原始失败、成功命令、协议、测量脚本及摘要见[D33回执](benchmarks/historical_size_acceptance_2026_10_04.json)。

同源码 `ca7813cba28d2abdef8309d2d3176893559d68c5` 的 [CI已22/22通过](https://github.com/loong10k/diskgraph/actions/runs/37158622289)。两个Windows Rust版本、Linux ARM及macOS Intel日志各实际执行全部28个选定D33回归（Core11/Engine9/FFI5/结构3），回执核对32源摘要与该提交一致。本次完成D33增量原生验收，当时Q04任务3.9因实际scope兼容性（D34）继续开放；Windows历史身份及历史正文绑定仍为独立未完成要求；本批不完成任何全平台父项，CLI/MCP危险写工具保持关闭。

## 实际历史命名空间（D34）

两侧均获授权不代表命名空间相同；合法已归属旧v1记录可具有不同无损根和相同显示字符串。正确回归记录为Engine **7/2**、隔离旧源扩展 **9/2**、FFI **5/1**（通过/失败）；初次APFS建目录错误与FFI标量断言错误明确不计缺陷证据。

Engine与旧FFI增长复用已有reader核对持久实际owner/server/scope，不同scope返回null增长，或保留 `different_root` 变化标签并新增 `scope_changed: true`。注册API保证scope根不可变；可信资格helper不授予权限，原响应预算与终态检查保留，包括Engine编码后复检。通用授权跨根比较和同scope已知旧历史仍可用；没有新增schema、依赖、owner、线程或修改导出。

最终源码本机workspace **1142通过/0失败/18 ignored，49 suites**。Engine命名空间 **11/11**，FFI新增 **6/6**、全部受影响增长用例 **20/20**；严格Clippy、定向fmt、include fmt、OpenSpec、构建和release通过，实际UniFFI **19/19**、stdio **18/18**、HTTP/SSE **13/13**。独立代码APPROVE、架构CLEAR，最终20源摘要一致；原始失败、成功日志及源码摘要见[D34回执](benchmarks/historical_namespace_acceptance_2026_10_04.json)。

Unix离线夹具以自然非法字节产生显示碰撞，Windows启用夹具以本平台自然未配对UTF16产生碰撞；均为公开API注册的合法合成元数据，不声称建过非法目录或执行过真实迁移。Linux另有2项真实原始目录用例，本机macOS未执行；现有socket验收运行过，但没有新增socket显示碰撞夹具。

同源码 `407125f62fda994826a7858737b22fa95efe4cb4` 的[原生CI已22/22通过](https://github.com/loong10k/diskgraph/actions/runs/37161135994)。两个Windows Rust版本和macOS Intel各实际执行全部 **17** 个选定用例（Engine11/FFI6）；Linux ARM另执行 **2** 项真实文件系统用例，共 **19** 项。20项已审源码摘要均与该提交一致；[D34回执](benchmarks/historical_namespace_acceptance_2026_10_04.json)保留终态元数据、原始job日志、实际执行用例名称和此前失败观察。

Q04任务3.9的完整C06/C07矩阵在下方D35继续，新源码仍需原生验收。D34完成命名空间增量原生验收，不完成全平台父项；命名空间相同不证明历史文件身份连续或正文版本相同。

## 原生输入准入与完整历史语义（D35，2026-10-04）

Git元数据和stash日志已知超限时先拒绝正文读取；每块同时受初始未读长度和剩余共享额度限制，终态核验长度及既有强身份/版本。reflog保留原OutputLimit分类和首次失败锁存；精确额度与空文件有效，管道EOF和清理契约未改。真实句柄偏移复现原4096字节及零长度超读，Git目标从4通过/3失败到8/8。

ScanCoverage以独立真实对象文件统一覆盖判定，字段、serde及公开导出不变。Core增长/变化/候选和Engine历史不能以complete标志覆盖不可读或深度缺口；Store原先就拒绝矛盾发布且继续整体回滚。Core真实RED为5通过/3失败，随后8/8；Engine10项涵盖六种设置双向变化、正负/零增长、未知/类型替换、卷/provider域值、partial、无重命名推断及真实宿主扫描增长/重命名。合成导入与opaque URI语义不证明挂载换卷或provider操作。

最终本机workspace **1168通过/0失败/18 ignored，51 suites**；构建、严格Clippy、fmt、OpenSpec及release通过。实际UniFFI **19/19**、stdio **18/18**、HTTP/SSE **13/13**及14项上游摘要通过。独立代码APPROVE、架构CLEAR与最终18源清单一致。[D35回执](benchmarks/native_input_history_matrix_acceptance_2026_10_04.json)保留两组真实RED、初次错误分类/Clippy失败及新鲜最终日志，不声称配对吞吐或RSS提升。

任务3.9已在2450ab1同源码原生CI37164196395终态22/22及四份日志各26项实际成功验收；Git8.7/15.13仍需授权私有工作树捕获、持久typed输入/能力上限、内容权限fence、证据发布及CLI/MCP验收。现有OpenSpec设计仅记录下一实施契约，未启用新collector。原生写、真实provider、GUI/GRDB/Room、mobile/设备、签名和生产父项保持开放。


任务状态复核：167项中137项已完成、30项开放（包含汇总父项，不等于29个独立漏洞）。8.6重新打开：可信库进程采样不等于已授权产品collector；尚缺应用安装实例采集、PID启动及资源身份绑定、来源持久化与原子revision发布的端到端验收。已验收的解析、预算和通用发布子能力保持完成。

## 受约束捕获与查询准备（D36）

Git 可信 scope 入口采用保留的原生根／组件句柄和独占捕获工作树，拒绝 scope 外依赖、链接、源修改和根路径链替换。私有捕获保留固定 status 命令的执行权限语义，任意命令不属于该契约。Windows 实现仍待本增量原生运行，真实 provider 不下载尚未验收；授权持久任务、能力上限和原子证据发布仍待实现。

TUI 原始准入采用所需字段投影、整帧累计读取预算和无 payload 续页探针。首末控制锁争用及时拒绝，交付须遵守原期限、实际归属和实时授权。入口从1182行减为43行，真实对象和逻辑分别落在模块文件中。候选准备也将快照覆盖头计入共享原始字节预算；到期且未观测头时返回类型化空 Deadline 与 `coverage_observed: false`，字节不足和真实存储故障仍是错误。新增公开 Rust 字段要求下游字面量补字段，旧 wire 字段与 UniFFI 签名保留。

首轮全 workspace 为1227通过、2失败、18 ignored，两处失败完整保留：非标准测试模块路径未通过源码门禁，覆盖头有界准备最初改变了旧到期候选结果合同。标准模块位置随后通过3项布局和5项文件读取回归，期限合同修复通过 Store 候选10项与 Engine 请求预算8项。最终源码 workspace **1233/0/18（51 suites）**、全部七项质量门禁与 release 构建通过，真实 release stdio **18/18**、HTTP/SSE **13/13**及 UniFFI 校验值 **19/19**通过；原生 CI 尚待完成。[Git](benchmarks/scoped_git_capture_acceptance_2026_10_04.json)与 [TUI／查询](benchmarks/tui_budget_acceptance_2026_10_04.json)回执分别保留真实阶段证据，本处不关闭平台父项。

Release 20k／200k 文件各4项检查通过，32次查询、4客户端；扫描0.503／5.169秒，查询p50/p95为9.048／11.323及8.492／10.921毫秒，数据库+WAL为43,442,176／436,916,224字节，单个CLI子进程最大RSS为44,433,408／263,274,496字节。RSS由macOS time逐进程测量，不是并发总量；非配对观察不证明提速或严格内存上限。最终源码清单 `67cbee45…` 包含等价私有导入拼写，以及保留旧图例导出路径的一处明确局部expect；两次严格Clippy失败均已归档。

D36 原生运行已结束：`e32214d` 的 [CI37171948796](https://github.com/loong10k/diskgraph/actions/runs/37171948796) 为20/22，两套Windows工具链的相同两项范围边界回归失败；Linux ARM64与macOS Intel实际通过64/64与65/65选定用例。[原生观察回执](benchmarks/d36_native_observation_2026_10_04.json)保留终态及原始日志；先前“待完成”描述属于当时状态。诊断统一及原生禁止替换/owner释放正控已修改，但新源码尚未通过Windows原生验收。D37真实socket回归复现远程Index/Sync任务在原始token到期后仍发布revision，修复进行中，不能声明生产就绪。


## 持久请求授权与认领预算（D37）

Index/Sync任务持久保存不可变请求来源、token能力上限和原始绝对到期时间，在入队/合并、严格认领、执行、staging及图库最后SQL后的commit前复验，续租不延长原始token。远程旧未知来源任务安全拒绝，可信本地兼容保留。控制库v6→v7升级使用一致性备份、原始字节准入及不可变授权记录；损坏来源返回InvalidGraph并停止本轮，需要管理员修复，不声称自动隔离继续处理。

真实socket排队后token到期、runner候选页及认领后准备计时均有实际RED→GREEN。原50ms预算任务在认领后持图库锁等待100ms仍成功发布；修复后同一测试返回BudgetExceeded/Failed，无staging、snapshot、revision或collector发布。原生协作取消仍可能短暂超时，不承诺硬I/O墙钟、RSS或跨库原子性。

开发工作区为**1273通过/7失败/18 ignored，54 suites**，7项全部属于尚未实现的CLI/MCP Git产品入口合同。精确暂存安全候选导出时未包含两个从未tracked的未来功能测试文件，**1270/0/18，52 suites**；原有测试和新增授权回归全部执行。未来测试原文保留在工作区，失败日志归档，不据此验收Git8.7/15.13。候选构建、严格Clippy、限定crate的fmt、FFI独立include格式、OpenSpec strict及release CLI/MCP/FFI通过；实际release stdio18/18、认证HTTP/SSE13/13、实际调用UniFFI公开校验值19/19通过。非作者复审确认34份安全来源及另2份Windows纠正，摘要全部一致。

[D37回执](benchmarks/durable_job_authority_acceptance_2026_10_04.json)保存两种候选范围、原始日志、摘要及边界。Release20k/200k各4/4检查通过；扫描0.477/4.396秒，查询p50/p95为7.001/9.744及6.925/8.676毫秒，数据库+WAL43,458,560/436,932,608字节。非配对观察，不声明提速，本增量未测RSS。修改后原生CI仍待完成，15.20及30项未完成实现/验收/父项继续开放。本机存在Android平台/构建工具，但无NDK和连接设备；仅macOS命令行工具，无iOS SDK/模拟器。这些环境观察不能替代设备验收。


## D37 原生结果与 Windows Clippy 纠正

`b680a1ce35a118d1b5c396c046eaa9a26193978f` 的 [CI37176087169](https://github.com/loong10k/diskgraph/actions/runs/37176087169) 已终态 **21/22**。Windows stable/1.97、Linux ARM、macOS Intel 的选定 workspace 阶段均实际通过39项授权/范围边界回归；对应总数为1131/0/16、1131/0/16、1259/0/18、1270/0/18。Windows stable 在测试通过后因三处严格 Clippy 检查失败，不能算完整流水线通过。D37回执已保存终态、四份原生日志及逐项执行清单。

后续仅改 Windows 尾表达式和两处等价整除判断；精确暂存候选的本机fmt、13项范围回归、workspace严格Clippy及all-target构建通过，新源码原生CI仍待。未来Git产品代码和CLI拆分尚未暂存或验收；15.20仍包含collector授权门禁，不关闭任何父项。
