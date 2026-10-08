# Windows 内置 PowerShell 验收兼容性

台式机实际预检发现系统 PowerShell5.1/.NET Framework 缺少 `Path.IsPathFullyQualified` 和 `Convert.ToHexString`。原脚本在合法绝对路径准入前即失败，妨碍真实worker夹具准备。

先在该Windows主机执行2项隔离合成镜像测试，复制正向用例实际因 `IsPathFullyQualified` MethodNotFound 失败（1失败、1通过），不将编译或环境探测称为行为绿灯。修复仅将完整路径判断改为根结构校验、SHA256十六进制格式化改为兼容API；保留原只读源句柄、CreateNew、Flush、摘要/尺寸复核及环境发布顺序。C:relative与根相对路径仍拒绝。

将相同修复源码临时传至该主机，使用真实powershell.exe执行2项测试，全部通过：准确复制含非文本字节的合成文件；摘要和尺寸一致；已有目标拒绝覆盖且环境不变；三种相对路径均拒绝。合成文件不是真实产品镜像，不能证明产品扫描权限或Windows生产验收。测试源码及候选脚本摘要、远程输出见 `docs/benchmarks/windows_powershell_worker_2026_10_08/receipt.json`。

本次未安装PowerShell、Rust工具链或系统服务。后续须将最终提交快进到该机，再从其真实Cargo构建结果准备worker并运行功能门禁。原精确MSRV1.97.0与全平台同SHA门禁保持未完成。

## 中文代码页与环境文件编码

在固定提交1ac266a0上，Windows完整workspace/all-targets构建实际通过（123.078秒）；随后MSVC以代码页936读取UTF-8中文C注释，产生C4819并被/WX拒绝。明确加/utf-8后保持/WX，实际构建并运行C0000000与C0000005退出夹具，两个32位状态均正确。

进一步实测发现PowerShell5.1的重定向将GITHUB_ENV写为UTF-16，UTF-8读取报UnicodeDecodeError。先新增明确UTF-8读取与路径绑定断言，实际红灯1/1；将回执Set-Content与环境Out-File都显式指定utf8后，三项原生测试全部通过，未通过移除中文注释或关闭警告绕过失败。

该轮完整Rust验证仍固定1ac266a0；已成功生成的实际worker与退出程序重新核验来源SHA、长度和SHA256后，从回执显式绑定测试环境。原编码失败及后续候选脚本修复证据分别保留，不冒称旧脚本已通过。最终脚本改动须随下一次提交验收，当前在执行的原生CI不取消。
