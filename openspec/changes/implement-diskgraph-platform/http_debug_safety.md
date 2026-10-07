# HTTP debug格式化安全

状态：实施中。认证和限流前debug格式化不能因为有效UTF-8正文panic。日志保留方法、路径及body字节数，不打印正文内容；方法/路径以Debug格式转义。原认证、限流及协议行为不改变。

回归覆盖119个ASCII后接中文（120落在字符内部）、空正文、Unicode路径及敏感正文不出现在日志。需要真实debug服务请求后仍可继续healthz的验收；单元回归不代替该项。

当前验收：macOS 单元回归 3/3；隔离 Linux 实际 debug 服务进程测试 3/3，覆盖 Unicode 未认证请求 401、随后 healthz 200、原进程存活及 SIGTERM/SIGINT 正常退出。候选尚未完成最终审查与同一提交全平台验收，不声明生产就绪。首次进程测试因观察器等待持久连接 EOF 超时；修正为按 Content-Length 读取后通过，该观察器失败不计为产品行为红灯。

最终结构门禁发现内联测试模块，已迁移为独立 http_debug_request_tests.rs，未改变运行逻辑。Linux/macOS结构检查各11/11通过，Linux迁移后debug单测3/3；Engine/MCP all-target Clippy通过。Linux库160/160、主程序7/7及选定集成38项通过；完整包首次受夹具缺失部署资产阻止，不声称完整workspace验收。
