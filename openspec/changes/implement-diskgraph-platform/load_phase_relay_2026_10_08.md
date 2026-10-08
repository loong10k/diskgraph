# 负载成功命令的阶段诊断保留

旧 Windows 完整诊断已设置 DISKGRAPH_SCAN_DIAGNOSTICS=1，但 accept-readonly-load.py 的 invoke 在成功时丢弃 CLI stderr，所以原回执只有 193.097 秒整体 scan 耗时，没有 worker/conversion/staging/publication 子阶段。这是验收诊断缺陷，不能由现有整体数字确定产品热点。

本次在显式本地诊断下只转存固定七类阶段名、节点数和毫秒。解析原 stderr 最后 128 KiB，每阶段最多保留最后一条记录，最多七条 JSON；普通 stderr、未知阶段、附带文字和非法数值不转存。截断可能丢失早期阶段，不补造缺失采样；此限制不是对子进程输出收集的整体内存上限。默认关闭，既有结果解析、异常、完整负载、正式 300 秒和独立诊断 900 秒期限均保持不变。

TDD：新成功转发测试在旧实现实际返回成功 JSON 但没有任何阶段记录，14 项中 1 项行为失败；补齐后 14/0，全部 load 相关测试 18/0，OpenSpec strict 通过。最初测试引用不存在的 MODULE.os 发生测试准备错误，纠正为对 os.environ 的标准 patch 后重新取得上述行为 RED；准备错误不计生产缺陷。测试模拟 CLI 成功输出验证过滤和有界转存，不证明 Windows 性能或原生扫描通过。

Intel 完整旧 SHA 日志另保存：695/6/13 ignored，五项扫描认领回归已由本地 8dd428f 候选处理，额外 display_initial_callback_can_reenter_control_and_revoke 在正控 result.unwrap 出现 BudgetExceeded；同测试在当前本机 1/0，不能据此排除 Intel 故障。终态任务重复执行 integration 19/1 同样待 8dd428f 原生验证。当前两 Windows 测试任务仍在运行，未取消、未触发替代验收。

证据：`docs/benchmarks/load_phase_relay_2026_10_08/`。不修改生产任务 checkbox，不关闭任何平台或性能父项。
