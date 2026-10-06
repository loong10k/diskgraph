# HTTP 请求行与 MCP 单值头的歧义拒绝

本项延续 MCP 传输身份与协议分发约束，不完成全平台父项。

## 验收行为

真实 socket 的请求行必须含恰好 method、target、version 三段；保留 HTTP/1.0 与 HTTP/1.1 合法请求、查询串及既有请求预算。缺失/未知版本、多余字段及控制字符不得进入分发。Mcp-Session-Id 与 MCP-Protocol-Version 属于单值会话/协议身份，重复字段即使大小写不同或值相同也返回 InvalidData，不采用第一值或最后值。

回归必须先确认原实现接收这些歧义输入，再实施；合法请求、原 framing 拒绝、XFF 代理追加语义与读取期限测试保持。标准依据：RFC 9112 第3节请求行和第5节字段语法：https://www.rfc-editor.org/rfc/rfc9112.html 。不据此声称已复现代理请求走私或权限越权。

## 本机证据

真实socket新增3项：旧代码1通过/2失败，新代码3通过/0失败。现有HTTP36项改前Git完整隔离源码与改后均29通过/7失败，精确失败集合一致，失败源为默认macOS扫描Unsupported。该组不是完整通过。源码结构9/9、格式通过；macOS候选只覆盖MCP http.rs、lib.rs及新测试，源数599→600，完整MCP门禁159→162并要求新3项实际运行。日志见 docs/benchmarks/http_request_ambiguity_2026_10_06。原生验收尚未完成。

## 请求对象的真实模块拆分

将HttpRequest及其header/query_param真实方法迁至独立http_request.rs，http模块显式pub use保留原公开路径和字段。中文文档说明原生Rust来源，保留查询值不解码、不解释成路径和重复查询取第一匹配的旧语义。新增结构及公开路径回归先红后绿；函数体和字段定义必须与原实现一致，不引入compat/stub或新依赖。不据此完成大型HTTP模块整体拆分或生产验收。

该对象已真实拆分：字段、derive、impl及两方法去除注释/空白后的源码逐字一致，公开路径与原始查询值/重复查询首值回归保持。结构先RED后GREEN 10/10，socket3/3、候选保护13/13及格式通过。候选同步601源，完整MCP162门禁不变。本项未完成其余HTTP对象拆分及原生全量验收。证据见 docs/benchmarks/http_request_module_2026_10_06。

## HTTP 配置与响应对象拆分

沿用 RT-10 和已有传输契约，将 HttpLimits、HttpResponse、ServerConfig 分别放入真实实现文件；http 模块显式重导出，保持 diskgraph_mcp::http 的原公共路径、字段、默认预算、JSON/通知/stream/text 响应及 legacy 默认关闭语义。先以真实模块缺失取得结构回归 RED，再检查拆分后的真实构造行为、既有 socket 安全回归和增量结构门禁。不得以空壳文件、include、wildcard 或 lint 豁免充数。本批不宣称其余网络策略对象或整个 HTTP 传输已满足架构门禁。

拆分验收：模块缺失真实RED后，结构11/11、原socket请求歧义3/3、归档保护13/13和格式通过。三个对象的字段、derive及方法实现去除文档和import后逐字节一致，公开路径实际编译调用通过；中文注释保留默认预算、无正文通知及legacy默认关闭语义。Mac冻结候选同步604源，其余源未改；这不是当前SHA的完整162项MCP/三平台产品验收。证据见 docs/benchmarks/http_configuration_modules_2026_10_06。
