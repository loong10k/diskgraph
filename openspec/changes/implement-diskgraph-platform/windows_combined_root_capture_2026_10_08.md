# Windows 根属性合并查询

状态：实验已撤回，不进入产品。原生效率红灯实际为 0/1，合并后原生扫描回归14/14、固定前缀验证1/1、原生Clippy通过。但Release 20k扫描76.103秒、根链58.074秒，对照原两次72.011/73.057秒与根链55.715秒，没有测得整体收益。夹具创建9.934秒、清理12.641秒也较原运行慢，单次比较不足以归因；不能声称合并查询导致回退，也不能仅凭调用数宣称优化。源码补丁与测试见 docs/benchmarks/windows_combined_root_capture_2026_10_08/；Release终态观察为completed/exit0，其保留尾部见release_observation.json，完整receipt与日志仍在远程root_capture_release目录。读取完整回执时远程连接暂时失败，未重复派发已结束的任务。

已精确撤销本机实验三个产品/测试文件的改动，保留原属性与名称校验；远程三个候选文件尚待连接恢复后精确撤回，不声称远程已经干净。

连接恢复后，已用原补丁执行逆向check及逆向应用，远程git status为空；完整Release回执、产物摘要、阶段记录与回滚状态见release_result.json。本地/远程均不保留此实验产品改动。

沿用 FS-02 的属性权限、完整 128 位身份、每次名称重新打开和任务门禁。根链检查的调用次数与先后位置保持不变；仅将同一句柄的 Basic/Standard 原生查询合为 FileAllInformation 固定前缀查询，身份仍单独读取 FileIdInfo。没有正文、目录枚举、删除或新共享权限，没有结果缓存。

验收：实际 Windows 同一目录句柄在目录内容变化后，合并捕获与原捕获所有字段完全一致；成功路径原生调用前后门禁从 8 次减少为 6 次。保留根/祖先替换、取消、撤权、Cloud Files 和全部 Windows 原生扫描回归。固定前缀必须已完整返回，成功或仅名称溢出可使用；短返回或其他错误回退既有捕获，任务检查错误原样传播。完整名称永不用于身份。

先用委托原捕获的入口运行效率红灯，再实现合并查询。原生通过前不将实现视为可用；需要实际 Release 20k/200k 和原始300秒门禁测量后才能声明性能收益。FileAllInformation 官方语义：https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntqueryinformationfile 。
