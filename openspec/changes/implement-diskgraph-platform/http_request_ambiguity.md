# HTTP 请求行与 MCP 单值头的歧义拒绝

本项延续 MCP 传输身份与协议分发约束，不完成全平台父项。

## 验收行为

真实 socket 的请求行必须含恰好 method、target、version 三段；保留 HTTP/1.0 与 HTTP/1.1 合法请求、查询串及既有请求预算。缺失/未知版本、多余字段及控制字符不得进入分发。Mcp-Session-Id 与 MCP-Protocol-Version 属于单值会话/协议身份，重复字段即使大小写不同或值相同也返回 InvalidData，不采用第一值或最后值。

回归必须先确认原实现接收这些歧义输入，再实施；合法请求、原 framing 拒绝、XFF 代理追加语义与读取期限测试保持。标准依据：RFC 9112 第3节请求行和第5节字段语法：https://www.rfc-editor.org/rfc/rfc9112.html 。不据此声称已复现代理请求走私或权限越权。

## 本机证据

真实socket新增3项：旧代码1通过/2失败，新代码3通过/0失败。现有HTTP36项改前Git完整隔离源码与改后均29通过/7失败，精确失败集合一致，失败源为默认macOS扫描Unsupported。该组不是完整通过。源码结构9/9、格式通过；macOS候选只覆盖MCP http.rs、lib.rs及新测试，源数599→600，完整MCP门禁159→162并要求新3项实际运行。日志见 docs/benchmarks/http_request_ambiguity_2026_10_06。原生验收尚未完成。
