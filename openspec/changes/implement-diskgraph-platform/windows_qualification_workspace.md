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
