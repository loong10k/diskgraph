# PF-06 受管宿主作用域生命周期

当前 `NativeServiceOwner` 拥有静态生命周期且 Drop 同步 join。不可 Send/Sync 不能防止安全 Rust 将其放入 thread_local；Windows TLS 析构持有 loader lock，等待另一线程退出存在结构性死锁风险。此判断不是当前 Windows 测试失败的运行时证明。

## 接口与所有权

新增 `NativeService::with_owner<R>(path, host: impl for<'host> FnOnce(Arc<NativeService>, NativeServiceOwner<'host>) -> R) -> Result<R, NativeServiceError>`。固定 R 与私有不变生命周期标记阻止能力逃逸。删除本轮新增且未导出 UniFFI 的 `new_with_owner`；原 new、stateless 与 19 项 wire ABI 保持。

私有 `NativeServiceHostGuard` 在库函数普通栈上拥有唯一 manager JoinHandle 和 lifecycle。公开 owner 仅借用该 guard，不提供公开构造、Clone/Send/Sync 或句柄导出。callback 的正常返回后显式 finalize 并传播清理错误；unwind 时守卫仍关闭和实际回收，再延续原 panic。

公开能力的显式 finalize 与提前 Drop 仍委托同一个 guard 完成真实 join；不能把早 Drop 降为只取消。重复 finalize 幂等。即便能力 mem::forget，函数仍保留独立 guard 并在 scope 结束回收。Service 可先释放，guard 不依赖 Service 强引用，退出后返回的 Service Arc 已关闭。

保持现有 coordinator manager、真实 join 共享状态、准入上限及 generation/fencing 机制，不新增全局 reaper，不 detach，不设置伪退出标志，不以 is_finished/TLS 消息代替实际 join。

## 验收顺序

1. 原 owning API 的安全 static TLS 存储编译成功作为结构性缺口证据，不把接口缺失编译失败当作行为 RED。
2. 保留已验收真实 TLS finalize 与 owner Drop 用例，迁移到 callback 后所有参与线程在 TLS 进入前启动并确认。
3. 增加真实 manager TLS 阻塞时忘记借用能力的运行回归：callback 的结果已产生，但 with_owner 不能在主动释放和实际 manager join 前返回。
4. 新 API 的正向外部消费者编译通过；返回 owner、static TLS 存储、static Fn/type erase、跨线程和嵌套作用域交换能力的反例因生命周期或 Send 约束拒绝，不因 API 缺失拒绝。
5. 正常返回、提前 Drop、显式 finalize、callback unwind、manager panic、Service 先释放和构造失败均按真实句柄/业务状态验收。
6. 保留原 19 项 UniFFI 校验值、FFI 回归及同 SHA Windows stable/MSRV 与三桌面原生 CI。独立审查后记录实际结果。

## 能力边界

类型系统阻止静态 TLS 保存 owner 能力；不识别直接从 TLS 析构、DllMain 或 UI 调用整个 blocking API。长生命周期宿主应在后台线程普通函数体内执行 callback 的命令循环。调用上下文和正式宿主包仍需原生验收，不据此勾选 9.2、9.8、15.4 或整个 PF-06。

此项只覆盖协调层 manager 和内部 Engine runner。上游 pinned scanner 的实际物理退出和源读取隔离、移动 provider、签名及设备验收保持未完成。


## Windows 扫描镜像名称绑定验收（未完成）

镜像摘要核验不能仅保留文件句柄：后续按名称加载还须绑定同一个本地 DOS 路径、逐组件非重解析父链和相对父句柄的末叶。GetFinalPathNameByHandleW 使用固定 32768 UTF-16 缓冲，拒绝溢出、UNC/未知 namespace、ADS、点组件，不做 lossy Unicode 转换或完整路径重开降级。获取原路径与父链、核验末叶及复查原句柄均消耗原请求绝对期限与原 checkpoint，不新建期限。

新父链租约集成到现有材料准入并保留到材料释放；验证前后核原父链及末叶完整身份/版本，占位或重解析明确拒绝。真实 Windows 回归须覆盖父目录改名/替换被阻止、释放后可改名、非 UTF-16 标量名称保真、父链建立中取消释放租约，以及原 SHA/写共享断言。原生未运行不得计为通过。

该绑定不冻结目录内新增 DLL，不证明既有可写映射、PE 依赖与加载器策略；Windows 扫描执行继续拒绝，待实际加载与整 Job 生命周期验收。单次原生调用仅有前后合作式期限检查，不承诺硬墙钟时间。

本阶段材料准入已保留父链并在摘要完成后验证原末叶与原句柄。实际本机 aarch64-apple-darwin：公开准入期限 2/2、配置 15/15、源码结构 6/6、相关 crate fmt 成功。Windows 新增三项行为未原生运行，TDD RED/GREEN 未证明；本机 cfg 未执行 Windows 实现，不能替代原生编译。现有 58dea21 CI 的 Windows Build 与 package 均仍被五组扫描接线 dead-code 阻塞，未关闭父项。证据：[候选记录](../../../docs/benchmarks/linux_test_boundary_2026_10_06/windows_image_binding_2026_10_06_candidate.json) 及同目录原始压缩日志。


## Linux 正常等待前的原授权检查

Linux 使用原子 pidfd/限制派生的线程组执行器，不为旧 Unix 数值组入口授予 macOS 正常回收资格。正常回收必须在确认整个原线程组、stdout/stderr EOF 与控制关闭之后、消费原 pidfd wait 之前再次执行同次原 checkpoint；此处撤权/期限错误须保留非 Clone 原错误、原未回收身份及 owner。消费后还须检查原请求，不能据此取消发布前的实时授权。

实际 Linux 新回归覆盖：两 EOF 但原进程仍活动、首检查失败保留原错误与存活线程组、第二检查失败不消费原 wait、独立两进程完成/非零退出码互不影响。固定 C fixture 仅模拟进程行为，不实现出生、filter、pidfd 或 wait 算法；root CI 编译并绑定实际 ELF。旧 Unix/macOS 五项正控保持，在 Linux 真实 pidfd 正控验收后再校正旧入口的平台负控，不以简单跳过关闭门禁。

此阶段仅提交四项原生回归及进程行为 fixture，尚未修复 wait-before-check 顺序。Windows cc4951d 原生 Build 无新增绑定编译错误，仍因原五组接线 dead-code 失败；绑定行为未执行。本机 macOS 旧正常退出 10/10、结构 6/6、fmt 通过，不算 Linux 原生通过。58dea21 Linux 完整日志确认 Engine 546 passed/6 failed/3 ignored、FFI 40 passed/49 failed，保留原始日志。新增前置 wait 授权回归须先在真实 Linux 观察目标断言 RED，再实施顺序修复；该阶段不关闭任何父项。记录：[RED 候选](../../../docs/benchmarks/linux_test_boundary_2026_10_06/linux_normal_contract_2026_10_06_red_candidate.json)。


Windows CI 在完整产品 Build 前增加正常 --lib 原生单元边界：执行现有八项镜像材料与两项实际目录清理回归，stable/MSRV 均沿用全局 -D warnings。未复制或屏蔽产品源码，不关闭 lint；完整 workspace Build、全量测试及 Clippy 仍是生产必需门禁。此前置运行仅为取得实际行为证据，不能把产品构建中的未接线能力声明为完成。

Linux e29153a 的实际 stable x86 CI run 37457793565/job 112249560997 中，新四案为 3 passed/1 failed。目标 final_pre_wait_checkpoint_failure_retains_original_wait_and_nonclone_primary 在明确断言“final authorization must precede original pidfd wait”失败，已证明原 wait 被提前消费，非编译/夹具/平台不可用失败。现于 reap_normal 之前补同次 checkpoint，同时保留方法入口及消费后原检查；修复后的 Linux GREEN 待同 SHA 原生 CI，不能以本机 macOS cfg 结果替代。旧六项 Unix 契约差异和 FFI 全量失败仍保持开放。


旧 Unix 非 macOS 正常接口在平台资格门禁处返回 Unsupported，尚未观察原 leader，因此不能把它解释成已经记录 ECHILD。真实外部 wait 消费后，首次兼容清理必须实际观察并保留原 ECHILD，立即撤销旧数值组清理资格；其后重复清理必须拒绝 Unsupported。原 macOS 已在正常观察发现失权时的断言不改。该测试校正不授予 Linux 旧正常回收能力，也不替代 Linux pidfd 正控验收。


Linux b20c88d 的 actual x86_64 stable 原生四案全部通过；原 RED 3/1 与 GREEN 4/0 已绑定同一固定 C fixture 源码与实际 ELF 摘要。按已验证的 pidfd 正控，旧 Unix 五项测试仍在 Linux 实际创建/观察原会话，但明确验证旧数值组 normal 的 Unsupported 资格门禁；Mac 保留原 active/后代/nonclone/等待前检查/独立会话正控及实际 normal wait，未增加 ignored 或按 cfg 跳过用例。Linux 旧测试释放后只使用受信异常清理真实等待，不声明为 normal 许可。新五项 Linux 平台负控与完整 Engine GREEN 仍待本次原生 CI。

Windows b20c88d stable 原镜像材料实际 7 passed/1 failed：父目录 rename 被拒绝为 ERROR_ACCESS_DENIED(5)，仅测试假设的 ERROR_SHARING_VIOLATION(32) 不符，非租约失效。修订同案加入租约前同一 rename 往返正控、原名称与目标缺名检查，租约期间仅接受 5/32 两种拒绝，释放后同操作须成功；叶写/替换的原 32 断言保持。修订后 native GREEN 待 CI，目录清理因前置失败尚未执行；不据此启用 Windows 扫描。

Linux 同一 b20c88d/37459156304 的 x86_64 stable、x86_64 Rust 1.97 与 ARM stable 三组原生四案均 4/0，已逐一核对固定 fixture 源码摘要，原始日志/实际 ELF 摘要保存在 [平台契约记录](../../../docs/benchmarks/linux_test_boundary_2026_10_06/platform_normal_contract_2026_10_06_candidate.json) 同目录。本次旧契约和 Windows 错误码校正，本机 Mac 原正常退出 10/10、结构 6/6、fmt 通过；Linux 全量/Windows 修订后的 native 结果仍未取得。

## Windows 产品清理正常路径正控（未完成）

必须在产品 `GitPrivateDirectory::with_limits` 创建、登记嵌套目录和文件后，调用原 `complete` 实际删除整个登记树，并验证原根缺失及重复完成幂等。保留移动原根与同名外来替换的原回归；改名在夹具阶段被租约拒绝不能代替清理成功。现有原生 `held_parent_lease_blocks_move_until_creation_phase_is_released` 明确证明创建父租约存活时改名被拒绝，释放后原根 anchor 仍可核验移动后的身份；产品仍复制保留创建租约，须取得正常清理原生证据后再调整生命周期。

1debd3e/37460438073 Windows stable 与 Rust 1.97 镜像材料八案均通过；产品目录清理仍在移动原根的夹具构造处失败，不授予执行资格。本次新增原路径正控尚未在 Windows 执行，不声称 RED/GREEN。该提交 Linux Rust 1.97 全量 Engine 556 passed/0 failed/3 ignored、Store 291 passed/0 failed/1 ignored，FFI 40 passed/49 failed；全工作区门禁仍未通过。

3833cac/37462037490 Windows stable 与 Rust 1.97 实际执行新增正常路径正控成功，产品三案均 2 passed/1 failed；原失败仍在移动原根时的分享冲突，尚未进入清理。长期持有创建父租约没有阻止产品正常清理，不能据此修改生产分享保护。必须进一步验证底层移动原对象清理及产品租约阻止替换的完整契约；当前不关闭清理父项。stable 原始日志及结果记录：[实际原生证据](../../../docs/benchmarks/linux_test_boundary_2026_10_06/windows_product_cleanup_original_name_2026_10_06_candidate.json)。
