# 显示消费期真实 token 到期夹具

2026-10-09：CI 37839056646、macOS Intel stable 的 `display_reader_refuses_token_expired_during_consumer` 在进入 consumer 之前已收到 PermissionDenied，未触达该测试声明的消费中到期行为。旧夹具只等待墙钟至少达到 500ms；若准备时已在 999ms，随后初始授权就可能跨过固定 token 秒边界。

修正只调整 revision/display 两个相同模式夹具：先完成 policy 准备，再从真实墙钟 500–550ms 区间发起一次原请求。sleep 后重读墙钟，错过区间则继续等待下一个区间；准备阶段最多五秒，无法获调度则显式失败，不无限等待。不重试授权请求，不刷新 token 到期时间，不增加原一秒查询预算。consumer 必须在固定到期前进入，等真实到期后仍在原查询期限内，最终结果仍必须 PermissionDenied。

macOS 本机及 Windows Rust 1.99 MSVC 的 `token_expired_during_consumer` 目标组各 3 通过；Windows 源码指纹和命令见 `docs/benchmarks/ownership_reader_2026_10_09/windows_expiry_and_mcp_diagnostic.json`。macOS Intel CI 尚待完成；本夹具修正不作为运行时授权或全平台生产验收完成证据。

CI 37849885654 的 b510f823 macOS Intel 完整 Engine 为 740 通过、1 失败、13 忽略；display 夹具在请求前未能于五秒内命中 500–550ms 的 50ms 准备相位，尚未调用被测请求。修正仅将可入场相位改为 250–650ms：固定下一整秒到期，真实剩余 350–750ms；不改变五秒夹具准备上限、原一秒查询预算、真实到期等待及消费前/后两项检查，不重试实际请求。sleep 后仍读真实墙钟，不让调度超时变成通过。macOS Intel 对应同 SHA 完整复验保持开放。

Windows Rust 1.99 原生任务 wc_job_gyhI0Qvs-igSWi86，修正后消费中真实到期目标组 3/3、格式检查通过，证据 `docs/benchmarks/windows_private_creation_9d09e1c_2026_10_09/native_expiry_phase.json`。该目标组与前一轮完整 Engine 分别绑定实际源码摘要，不能冒充修改后全量绿色。
