# Windows 原生验收的完整 workspace 隔离

本项延续 15.13 原生证据门禁，不完成父项。

## 验收要求

对旧版对照、新版候选和普通退出基线分别导出冻结 manifest 指定的完整 Git commit，再覆盖逐文件摘要校验的现有候选归档。不得将旧 Cargo.lock 写入当前产品 checkout，不得去掉 --locked 或改写归档来掩盖依赖差异。原目标 RED、65 个候选用例与结果核验保持不变。CI 先取得精确历史 commit，再执行所有隔离测试。

## 证据

CI 37410205333 在旧版对照编译阶段实际失败，错误为 cannot update the lock file；新版用例跳过，不计通过。新增隔离回归先 RED 后 GREEN；三份完整导出均通过真实 cargo metadata --locked 全依赖解析。分别注入当前 CLI manifest 后均真实复现锁文件拒绝。归档逐文件摘要保持一致，产品 checkout 未修改。Windows 驱动 22 项和挂载保护 13 项本机通过。日志见 docs/benchmarks/windows_workspace_isolation_2026_10_06。

该记录不证明 Windows 原生编译、65 项运行或全平台生产就绪；等待新的真实 CI。

## Windows 换行转换的真实回归

CI 37411176883 在驱动单元测试发现完整导出受 core.autocrlf=true 影响，与原 Git blob 的 LF 字节不一致。保留逐字节断言，本机通过真实 Git 环境设置复现三份快照 RED。导出命令显式关闭 autocrlf 并设 core.eol=lf 后，三份精确源码及完整依赖/混合 manifest 反例均通过，驱动22和挂载13通过。原始 CI 与 RED/GREEN 日志见 docs/benchmarks/windows_archive_eol_2026_10_06；不计原生用例通过。

## 当前原生终态：64/65

CI 37411402086 的真实旧版目标 RED 对照通过，新版65个精确用例实际运行：64通过，1失败。两个移动根正控、父目录创建期lease阻止移动及解除后成功均通过，原先三个根失败关闭两项。剩余 original_share_read_lease_blocks_delete_reopen_until_released 仍失败：完整原对象身份相同，按名 DELETE open 拒绝32，按ID重开获得0x110081含DELETE。不能由此推断实际删除会成功。原始两份receipt及相关日志见 docs/benchmarks/windows_native_64_of_65_2026_10_06。

诊断只在本案独占空临时目录的已核身份原句柄上尝试 FileDispositionInfo，再在同句柄关闭前立即撤销标志。所有原测试断言和生产方法保持；不使用POSIX删除、DELETE_ON_CLOSE或用户路径，不启用写工具。SDK依据：https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfileinformationbyhandle 。原生结果待后续CI，不能计通过。当前默认Engine严格Clippy重跑仍42诊断，不完成产品默认接入门禁。

## 按ID实际删除标记诊断

CI 37412012814 原生仍64/65，唯一原共享租约断言失败。实际同句柄 FileDispositionInfo 返回87，未成功标记删除；按名 DELETE open 仍32，按ID open虽获DELETE但不能据此声明可用删除能力。原型尚未接入产品目录清理，不放宽原断言，不启用写工具。receipt及实际日志见 docs/benchmarks/windows_id_disposition_87_2026_10_06。下一步必须找到并原生验证无路径逃逸、身份及共享语义一致的句柄执行方式。

## 原句柄空名称重开候选

已复现的64/65原生失败作为本次RED。候选使用NtOpenFile，RootDirectory为原anchor，ObjectName为零长度UNICODE_STRING，要求DIRECTORY、同步、OPEN_REPARSE_POINT及OPEN_NO_RECALL，不解析客户端或当前路径，不从名字/ID重新发现对象。重开前后仍核验原anchor及新句柄完整卷/file ID身份。参考Go标准库原始实现：https://github.com/golang/go/blob/master/src/internal/syscall/windows/at_windows.go 。该参考不能代替Windows原生验收。

原共享租约错误32、移动根/恶意替换/目录联接正控保持。租约释放后新增同一原句柄可撤销删除标记正控。旧按ID诊断保留为独立对照，不计作可用删除能力。原型未接入实际目录清理、增量walker或Pool，不启用任何文件操作工具。

本候选已实现，生产原型和冻结候选的两个根文件完全一致，其余495源字节保持。结构6/6、驱动22/22与格式通过，仅证明静态及验收驱动能力；本机Mac未执行Windows方法。原按ID诊断迁移为单独夹具对照，原租约错误32断言不变，解除后必须实际mark并clear。证据见 docs/benchmarks/windows_anchor_relative_reopen_2026_10_06；等待真实原生CI，父项继续开放。

## 目录重开选项兼容性修复

CI 37412964453 已终态失败：65个原生用例中60通过，5个根重开场景返回错误87。该结果为本次RED，不算生产验收。微软NtCreateFile的CreateOptions规则限制FILE_DIRECTORY_FILE的兼容标志；原候选将其与FILE_OPEN_REPARSE_POINT及FILE_OPEN_NO_RECALL合用。修复重开选项组合，保留OBJ_DONT_REPARSE、OPEN_REPARSE_POINT、OPEN_NO_RECALL和同步访问；目录类型由原anchor及返回句柄的完整身份与显式目录检查确认。新建目录仍使用FILE_CREATE和FILE_DIRECTORY_FILE。

验收保持全部65项、共享租约期间错误32、解除租约后的真实mark/clear、移动与恶意替换正控。不得以按名回退、关闭防护、删除断言或启用产品清理来替代。依据：https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/nf-wdm-zwcreatefile 。原生结果未返回前仅记候选修复，父项保持开放。

原生结果：CI 37413671391 在 e4bc1f0b4e60c6f255b00ca07c26ffc590e64902 实际通过全部65项；逐个stdout核对精确用例与1 passed/0 failed，没有零用例通过。旧版目标RED对照通过。此前五项根重开失败均关闭；共享租约期间仍拒绝32，解除后的原对象句柄实际mark/clear成功，按ID对照仍返回87。证据见 docs/benchmarks/windows_directory_open_options_2026_10_06。本结论仅证明原子根句柄子层，不完成增量删除walker、Pool、可信安装、默认Windows扫描或全平台生产门禁。

## 原句柄分页目录枚举

下一步接入 Windows GitDirectoryLease::read_names：只从原 lease 句柄枚举，调用方路径不作为重新打开依据，与现有 Unix 语义一致。使用 FileIdExtdDirectoryInfo/RestartInfo 的64KiB固定对齐页，逐项保留完整128位ID和原UTF-16名称；检查记录长度、偏移进展、名称单组件及非零ID，未知能力明确失败，不退回read_dir/按名枚举。每页原生调用前后和每项均检查原 ProbeBudget，不续期。元数据条目/字节账本保持，整个 names 返回仍受原32768项预算，不声称它已成为扫描流式或严格RSS能力。

回归：原lease绑定A而参数路径指向B时必须只读A；宽目录跨多页及非UTF-8名称保持；非法原生记录拒绝。原路径枚举以同一新测试、唯一替换的已提交旧GitDirectoryLease源码取得实际Windows目标RED，编译失败不算；修复与原65项一起运行，不减断言。此层是后续原句柄cleanup walker的枚举基础，不完成整个目录删除或产品启用。依据：https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_id_extd_dir_info 。

本机结构6/6、验收驱动24/24、归档保护13/13、格式和OpenSpec严格校验通过。Windows清单497源，保持原65项并增加两个真实文件系统用例及一个记录解码用例，共68项；Mac606源同步枚举模块，MCP162及原生child41的门禁不减少。旧枚举源码逐字节等于888fbc2中的已提交文件。Windows目标RED与68项实际运行尚待CI，不以本机cfg排除当Windows通过，也不勾选15.13父项。证据见 docs/benchmarks/windows_directory_cursor_2026_10_06。

## 原生验收驱动的编码修复

CI 37415041940 在归档保护步骤实际失败：新增模块检查使用Windows默认cp1252解码含中文的UTF-8 Rust源，触发UnicodeDecodeError；原生用例均未运行，不计68项失败或通过。用同一真实归档、仅注入默认cp1252读取方式，本机复现相同位置的RED。源码、UTF-8 Cargo输出和receipt显式按UTF-8读取，不依赖PYTHONUTF8或变更系统locale，不用errors=ignore/replace丢字节，也不放宽归档摘要或模块检查。修复后重新运行实际Windows目标RED和68项候选，未返回前保持待验收。

原生终态：CI 37415666200 在0ae9b18f4d75951eb9c3f6eee4ffdbd496c64e6b通过全部68项。已逐个stdout确认精确名称与1 passed/0 failed；原65项保留并通过。旧枚举精确目标实际RED（原lease A却返回B的sentinel），修复后只返回A的owned；1201个原始名称跨页及每项完整128位ID实际匹配，其中包含未配对UTF-16单元。解码边界用例是软件记录验证，不当成额外内核文件系统能力。编码驱动障碍在Windows实际关闭，归档保护通过。见docs/benchmarks/windows_directory_cursor_2026_10_06及windows_qualification_encoding_2026_10_06。仅关闭原句柄枚举子层；尚未接入整个cleanup walker/Pool、默认Windows扫描、可信安装或全平台同SHA发布门禁，15.13父项保持开放。

## 原父句柄下的清理子项核验

以已验收游标提供的完整128位ID为预期，在原持有父句柄下按单组件NtOpenFile打开；不从原/当前绝对路径或按ID重新发现对象。要求原父及新子项同卷、类型和完整ID一致，打开前后检查原父身份与原ProbeBudget。保留OBJ_DONT_REPARSE、OPEN_REPARSE_POINT、OPEN_NO_RECALL，不混入不兼容的DIRECTORY_FILE选项。目录只增加LIST_DIRECTORY；普通文件只请求属性、同步及DELETE，不申请内容读取。原生共享冲突保持原错误，不弱化shareREAD租约。

新增原生正控/反控：父目录移动并被陌生同名根替换仍只开原子项；枚举后同名子项被替换则拒绝；实际junction替换不跟随且外部sentinel不变；活跃shareREAD文件租约阻止DELETE打开，解除后原ID通过。这是新增内部API，现有游标没有旧对应入口，不伪造旧版本行为RED；新增验收调用并实现，依靠真实正控和反控证明能力，原目录枚举目标RED仍保留。此子层不执行删除、不启用危险工具、不宣称整个walker/Pool或默认产品启动完成。

实现候选：open_verified_child在同一父游标下取得未执行删除的句柄，预算/原父/新子项的失败不改变游标位置，不覆盖或删除任何对象。新增ID比较不截断。GitPrivateAllocation超过500行触发真实结构RED，已将真实NtCreateFile工厂拆至WindowsGitChildOpen（不是空壳），方法体去空白逐字节一致，结构6/6通过。Windows498源、72项清单保留原68；Mac607源保持MCP162/原生child41门禁。驱动24/24、归档保护14/14、格式及OpenSpec严格验证通过；Mac本机未运行四项Windows原生用例，旧枚举行为RED仍需同一CI复验，删除、walker/Pool和产品启用继续未完成。证据见 docs/benchmarks/windows_verified_cleanup_child_2026_10_06。


### 原句柄实际删除与last-close候选验收

在已有同名陌生根替换原生测试中，进一步对已核完整ID的原子项句柄执行FileDispositionInfo，关闭后确认原子项消失；随后对原根anchor相对重开句柄标记删除，并释放全部原根句柄后确认移动后的原根消失，陌生根哨兵必须保持。新增精确stdout标记DG_VERIFIED_CHILD_AND_ROOT_LAST_CLOSE_DELETE=1。不使用POSIX、路径删除或delete-on-close回退；仅隔离测试，不接通产品删除、Pool或恢复，不启用危险工具。Windows实际结果待CI，不据此勾选15.13。

删除验收进一步拒绝exists()把访问拒绝当缺失的弱证据：最终symlink_metadata必须明确NotFound且原生错误2/3，pending/权限/其他错误不得计作回收完成。驱动必须同时验证精确案名、1passed/0failed/0ignored及最终断言标记；驱动新增回归真实缺方法RED后25/25GREEN，冻结挂载14/14。实际Windows结果仍待验收。

CI37417786621在3e50b4e原生终态success，72/72精确案stdout逐项核对实际1passed/0failed，新增四项原句柄子项回归均通过，两个旧实现RED回执同时保留。证据见windows_verified_cleanup_child_2026_10_06/native_acceptance_37417786621.json。该源码没有实际last-close删除断言，不能替代后续afef3c5/a8ed3dd验收或产品walker/Pool集成。

CI37417917011在afef3c5原生终态success，72/72stdout及测试源码SHA核对完成，实际子项+原根last-close删除标记存在，陌生根哨兵保持。本版本缺失检查仍用exists，不能证明a8ed3dd的明确NotFound门禁。实际libtest stdout将第一条println附在test案名前缀同行；驱动精确标记解析因此需兼容完整案名前缀+完整标记两种格式，新增真实格式回归26项中1项RED后26/26GREEN，错误标记=10仍拒绝；修正驱动已逐项接受该轮72份原始stdout，不忽略或替换任何原始日志。证据见windows_last_close_delete_2026_10_06。

### 最终删除的外部句柄反控

owner/Pool接入前增加真实外部shareALL目录句柄。对原根标记删除并释放本进程全部原根句柄后，以原可信父句柄为卷提示、原完整128位ID作只读存在探测：外部句柄未释放必须拒绝为ERROR_ACCESS_DENIED(5)，不能完成回收；释放外部句柄后必须原ID明确ERROR_FILE_NOT_FOUND(2)，移动后的原名称NotFound，陌生替换根sentinel保持。OpenFileById只作确认，不用于删除；未知结果保留恢复责任，不按路径或截断ID降级。对应官方合同https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-openfilebyid 。新增候选原生案尚未执行，不据此声明owner/Pool完成。

CI37418172143在a8ed3dd终态failure，保留原71/72驱动失败回执。逐份实际stdout为72/72原生成功，明确NotFound2/3断言通过；唯一驱动失败是首条println附在精确libtest案名前缀同行，修正驱动重验全部72份stdout接受。此轮不改写为workflow成功，证据见original_failure_revalidated_37418172143.json。外部句柄新增案加入精确清单后共73项候选，本地驱动缺标记真实RED后27/27GREEN，冻结挂载14/14、fmt/OpenSpec通过；实际外部句柄案尚未运行。

原生最终确认逻辑提取为实际WindowsGitDeletionWitness对象：同卷hint核验、原完整ID只读OpenFileById、未知/访问拒绝原错误、成功句柄完整身份与共享预算首末检；仅明确错误2返回absent=true。新原生外部句柄案直接调用本接口，根仍存在时false、pending时错误5、最终关闭后true及陌生根保真。冻结Windows499/Mac608源码，73精确Windows候选案；source_layout6/6、驱动27/27、挂载14/14、fmt/OpenSpec通过。尚未原生执行新对象，未接入owner/Pool，不关闭父项。

CI37418549680在27f6bf7终态success，72/72实际精确stdout与冻结源SHA核对通过，最终NotFound2/3及完整删除标记门禁均通过。只关闭上一轮驱动格式缺陷，不涵盖本轮新增外部句柄/WindowsGitDeletionWitness接口或owner/Pool。证据见native_acceptance_37418549680.json。

### 清理游标保留当前子项

新增open_next_cleanup_child/confirm_cleanup_child_absent：把原枚举名称、完整ID和核验身份保存在同游标中，原打开失败/删除等待/预算失败均不消费当前项；普通next_entry不得跳过未完成项。仅原ID最终明确消失才清除当前项。后续walker必须在删除前核对owner登记账本，本API不收养未登记对象。新增真实共享错误32、外部句柄delete-pending错误5、同ID重试、最终确认后才推进下一项及真正EOF回归；原生结果待CI，不完成整个walker/owner/Pool。

CI37419047031在7b2b4ef终态failure，72/73原生通过；新外部句柄案在std::fs::rename原根处错误32，尚未进入最终确认接口。原shareREAD父租约被夹具保留到移动阶段，修正为仅隔离temp固定父目录同身份shareALL卷提示，释放原创建/捕获lease后再移动。保留全部移动/陌生根/外部句柄pending5/最终2断言，其他原共享冲突测试不变。生产确认接口仍不解析路径，夹具路径打开不是产品授权或清理能力。原始失败见windows_deletion_witness_parent_lease_2026_10_06；新结果待原生，不能计作接口通过或walker/Pool完成。

CI37419410021在8cf92c6终态failure，72/74通过：父shareREAD夹具移动错误32之外，新游标案在外部句柄关闭后的原ID确认处实际OpenFileById错误87。等待删除时原错误5已实际观察，但87不是对象消失，不放宽断言或改判成功。原失败完整保留于windows_native_full_id_witness_2026_10_06。候选改为SDK NtOpenFile FILE_OPEN_BY_FILE_ID完整16字节对齐二进制名称，保留nofollow/norecall/sync与全身份/卷、预算核验，无路径或64位回退。以仍保活hint自身完整ID作同协议正控并复核完整身份，协议不支持/解释不明必须Unsupported，防止把未知ID语义的NotFound当成实际删除。NtOpen原错误87/5/其他仍失败；明确2才可absent。依据https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntcreatefile ，实际支持及最终缺失语义待新原生验收。
