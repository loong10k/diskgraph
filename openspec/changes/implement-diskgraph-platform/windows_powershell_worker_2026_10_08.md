# Windows 内置 PowerShell 验收兼容性

台式机实际预检发现系统 PowerShell5.1/.NET Framework 缺少 `Path.IsPathFullyQualified` 和 `Convert.ToHexString`。原脚本在合法绝对路径准入前即失败，妨碍真实worker夹具准备。

先在该Windows主机执行2项隔离合成镜像测试，复制正向用例实际因 `IsPathFullyQualified` MethodNotFound 失败（1失败、1通过），不将编译或环境探测称为行为绿灯。修复仅将完整路径判断改为根结构校验、SHA256十六进制格式化改为兼容API；保留原只读源句柄、CreateNew、Flush、摘要/尺寸复核及环境发布顺序。C:relative与根相对路径仍拒绝。

将相同修复源码临时传至该主机，使用真实powershell.exe执行2项测试，全部通过：准确复制含非文本字节的合成文件；摘要和尺寸一致；已有目标拒绝覆盖且环境不变；三种相对路径均拒绝。合成文件不是真实产品镜像，不能证明产品扫描权限或Windows生产验收。测试源码及候选脚本摘要、远程输出见 `docs/benchmarks/windows_powershell_worker_2026_10_08/receipt.json`。

本次未安装PowerShell、Rust工具链或系统服务。后续须将最终提交快进到该机，再从其真实Cargo构建结果准备worker并运行功能门禁。原精确MSRV1.97.0与全平台同SHA门禁保持未完成。
