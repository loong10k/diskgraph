# 最终响应回调期间的负向撤权

实际缺陷：finalize_revisions_read_until 只读取回调前后持久状态，没有建立原请求主体与各侧 scope 的负向见证。回调通过独立真实控制连接 revoke_grant 后 upsert_grant 恢复，返回旧 PolicyAuthorizer 的 Allowed，原实现实际返回 Ok(true)。新的单项回归确认回调确实执行一次且实时 grant 确实恢复，旧实现 0/1，候选 1/0。

实现：回调前为各侧建立 RequestWithdrawalWitness，保留授权代次；回调后先检查已知负向事件，再沿原固定 SQL 观察窗口核验所有侧当前权限和新鲜 revision 归属，结束后再次检查见证。已知撤权报告 PermissionDenied；无原生通知能力时，未知授权代次变化保守 Conflict。负向事件不因权限恢复或 SQL 预算错误而消失；能力回调、SQL 50ms 窗口和数据期限均不放宽。MCP relation_reply/snapshot_reply、FFI native_reply/native_growth 调用此既有终态入口，wire 与调用签名保持。

验证：新增回归 1/0；终态能力 14/0；FFI native_reply_deadline 4/0；Engine 结构 6/0、all-target Clippy、fmt、OpenSpec strict 通过。首次 Clippy 指出测试 Permission 为 Copy，改为解引用后通过。关系请求回归实际 9/8/0：8 项在夹具准备处返回 Business(Unsupported)，对应本机未安装可信受管 worker，不能当作通过；旧 Windows MSRV 失败的 expired_envelope 测试在本机实际通过，但不替代 Windows 复验。

原始红绿及回归日志和源码摘要见 `docs/benchmarks/final_response_withdrawal_2026_10_08/`。这是一项本机已复现且修复的授权观察缺陷，不证明完整远程传输或全平台生产就绪。5dc825d 的 CI 37744394886 已开始，不包含本增量；保留其真实运行，待终态后再推送本增量验收。不改任务完成勾选。
