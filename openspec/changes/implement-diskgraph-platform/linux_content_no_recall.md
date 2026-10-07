# Linux 内容不召回能力边界

状态：发现生产代码缺口，实施与原生验收未完成。沿用 CT-02，不改上游 pin 或 vendor。

当前 Linux HydrationGuard 返回成功的空策略，ConservativeProbe 对未知返回 false，Unix ScopedContent 随后打开数据句柄。静态证据不能证明已触发下载，但这条路径未满足无法可靠验证时 Unsupported 的既有要求。

占位诊断必须区分占位、可靠本地与未知；正文读取必须有绑定原资源/provider 的可验证不召回能力，缺失时精确 Unsupported，不回退普通 open/read。保留普通本地文件功能，不以文件系统 magic 或 overlay 名称自行签发能力。

验收：未知能力不进入数据打开；原资源能力匹配后本地正常读；不匹配拒绝；真实按需 provider 的产品读/哈希不触发内容请求，未保护读取正控必须触发回调。模拟探针不替代 provider。

隔离 Docker 内核有 FUSE 和原生头文件；本轮无需安装包，私有 /dev/fuse 在临时容器内可打开。后续原生 FUSE 夹具仅挂载容器私有目录，保留原 daemon、计数和实际卸载回收；不安装宿主服务。可用性探针不是产品验收。源码摘要见 docs/benchmarks/linux_content_no_recall_audit_2026_10_08。

## 原生 RED（2026-10-08）

无需安装libfuse，使用内核FUSE协议与原生头文件建立私有提供方。普通读取正控触发READ内容回调，随后真实Engine授权scope及ContentRead grant下，读/哈希两项均因产品触发内容回调失败0/2；产品两次与普通正控第三次计数均见原始日志。正常umount2及原daemon wait退出0已确认，没有lazy卸载或以杀进程冒充正常回收。

这将静态风险升级为实际生产缺陷；下一步需在申请数据句柄前拒绝没有验证不召回能力的资源。仅FUSE magic识别能修复这一已复現路径，不能被称作所有Linux provider完整能力。保持本地普通文件与原资源绑定语义，不修改原有授权/预算标准。

## 有限修复 GREEN（2026-10-08）

Linux 路径遍历改为 O_PATH，先在原句柄上拒绝已知 FUSE，再经 /proc/self/fd 获取绑定同一 dev/ino 的数据句柄。真实提供方产品读/哈希 2/2 通过，无 DATA_OPEN 或 READ；普通读取正控触发两种回调，正常卸载与原 daemon 回收通过。原路径替换与链接句柄测试 2/2、content_flow 7/7 通过。隔离工具链没有 cargo-clippy，本次 Linux Clippy 未运行，不以其他平台检查替代。

此修复只封闭已复现 FUSE 路径，其他 Linux provider 的可靠能力判定仍未实现，CT-02 全平台验收继续保持未完成。原始 RED 的夹具只有 READ 计数，后续 GREEN 增加 STATFS 和 DATA_OPEN 诊断；不把两份不同诊断夹具宣称为完全相同输入。生产与夹具摘要、原始日志见 receipt.json。
