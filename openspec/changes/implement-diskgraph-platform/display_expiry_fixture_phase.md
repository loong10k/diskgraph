# 显示消费期真实 token 到期夹具

2026-10-09：CI 37839056646、macOS Intel stable 的 `display_reader_refuses_token_expired_during_consumer` 在进入 consumer 之前已收到 PermissionDenied，未触达该测试声明的消费中到期行为。旧夹具只等待墙钟至少达到 500ms；若准备时已在 999ms，随后初始授权就可能跨过固定 token 秒边界。

修正只调整 revision/display 两个相同模式夹具：先完成 policy 准备，再从真实墙钟 500–550ms 区间发起一次原请求。sleep 后重读墙钟，错过区间则继续等待下一个区间；准备阶段最多五秒，无法获调度则显式失败，不无限等待。不重试授权请求，不刷新 token 到期时间，不增加原一秒查询预算。consumer 必须在固定到期前进入，等真实到期后仍在原查询期限内，最终结果仍必须 PermissionDenied。

macOS 本机及 Windows Rust 1.99 MSVC 的 `token_expired_during_consumer` 目标组各 3 通过；Windows 源码指纹和命令见 `docs/benchmarks/ownership_reader_2026_10_09/windows_expiry_and_mcp_diagnostic.json`。macOS Intel CI 尚待完成；本夹具修正不作为运行时授权或全平台生产验收完成证据。
