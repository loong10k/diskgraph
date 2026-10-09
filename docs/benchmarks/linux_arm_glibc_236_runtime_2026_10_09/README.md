# Linux GNU 运行库兼容性缺口 / GNU runtime compatibility gap

源码 `98455bcf` 的 ARM64 CI归档摘要已核对；使用现有、固定摘要的
glibc 2.36 ARM64容器，在UID1000、无网络、只读文件系统和关闭全部
capabilities的环境执行归档内真实 `diskgraph --version`，加载器拒绝
`GLIBC_2.39`，退出1。未创建数据库或扫描，未安装宿主工具或服务。

ELF版本需求与实际摘要见 `abi_guard_receipt.json`：CLI和MCP要求最高
2.39，scan worker最高2.34。CLI动态符号中 `pidfd_spawnp` /
`pidfd_getpid` 绑定2.39；`statx`绑定2.28，`memfd_create`绑定2.27。
这是实际制品的导入证据，不用弱符号名称推定旧加载器可接受。

当前 `package-linux.sh` 生成的私有bundle清单声称glibc最低2.17；
本次CI GNU归档的运行库需求不能支持这一承诺。检查器对三份归档
镜像均拒绝2.17声明，并接受各自真实最高版本作为解析正控；解析
正控不是旧系统运行成功。原缺失脚本的初次测试在加载阶段失败，
没有测试执行，不称行为RED；最终9项格式/数值/拒绝回归通过，
其中错误ELF对象类型的三种实际失败已独立保留为检查器RED。
原制品的真实RED是上述加载器失败。新工具尚未接入打包流水线，尚未
生成修复后的制品，兼容性门禁保持未完成。

后续需取得旧glibc构建SDK并重新构建，不移除版本表或伪造符号。
直接 `statx`/`memfd_create` 包装依赖和构建时链接需求都须处理；
现代内核的原生身份、pidfd、安全镜像和授权检查不能降低。旧运行库
与实际内核安全能力分别验收。manylinux2014容器的2.17基线及ARM64
镜像由 [PyPA 官方说明](https://github.com/pypa/manylinux)确认；候选镜像
清单摘要为 `9026a55e05e76ad74d9a4e6be814a5abda26cde08b1baa659ea4f7ee16b246e3`，
只查询了清单，尚未下载/准备SDK，等待新增工具安装授权。

The exact ARM64 CI package failed to execute `diskgraph --version` in an
isolated glibc 2.36 runtime: the loader requires `GLIBC_2.39`. CLI/MCP need
2.39 and the worker needs 2.34, contradicting the private bundle script's
2.17 compatibility claim. Original image hashes and the failure are retained.

Nine local parser tests pass. Invalid ELF object kinds failed before the
guard was added, independently of the package loader failure. The inspector rejects the 2.17 claim for all
three actual images; this is a diagnostic, not a rebuilt package or old-runtime
qualification. It is not integrated into packaging yet. Fixing the build
baseline and native wrappers must preserve kernel safety and authorization.
No version-table patch, invented symbol or unsupported fallback is accepted.
