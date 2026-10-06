# Origin 边界修复 / Origin boundary hardening

已复现三个真实 RED：带路径 localhost Origin 被当作合法来源、无端口 IPv6 回环被拒绝、缩写或错误括号 bind host 获得本地信任。修复采用完整 authority 校验、标准 IP 解析及 ASCII OWS；不截断路径/查询/fragment/userinfo，不通过允许列表收养 malformed Origin，保持正常端口与允许列表精确比较。

Three actual RED tests cover path truncation, valid portless IPv6 rejection, and malformed bind hosts receiving local trust. The fix validates the complete authority, parses IP addresses through the standard library, and trims only ASCII OWS. Configured remote origins still compare exactly.

六项精确本机行为实际通过，包含四条真实路由的20次正反观察：POST /mcp、GET /mcp、GET /sse、POST /messages。malformed Origin在认证前403，合法回环但缺token为401；使用显式stop/join关闭原listener和线程。不将未认证拒绝称作授权工具执行验收。

Six exact local cases passed, including twenty real socket observations across four routes. Malformed origins return403 before authentication; valid loopback origins without a token return401. The runtime is explicitly stopped and joined. This subset does not establish authenticated tool execution or full production readiness.

保留最初候选仍允许[localhost]的失败及扩展过滤12/1结果；后者扫描夹具在Origin路径前Unsupported。六项修复不替代Engine默认扫描、完整workspace/严格Clippy或三平台门禁。新增手动三平台CI在当前产品源码上编译并精确执行六案，按源码内容及二进制SHA绑定结果；原始stdout/stderr、零执行及ignored均不能冒充通过。

The initial candidate failure for [localhost] and the expanded12/1 result remain preserved. The latter failed in an unsupported scan fixture before Origin handling. Default scanning, full-workspace quality gates and three-platform acceptance remain open. The new manual workflow builds the current checkout and records exact-case logs, runtime source fingerprints and the executable digest.

追加：真实HTTP字段名/控制字节案与Unicode空白原始socket案也确认目标RED。最初Cursor夹具接口类型错误的编译失败单列保留，不计行为RED；改用实际TcpStream后才取得失败证据。最终七案实际通过、四路28次观察；token字段名按HTTP tchar校验，值只移除ASCII OWS，非ASCII不被静默抹去。MCP结构11通过，package no-deps严格Clippy通过；依赖Engine的42warnings与完整workspace未完成状态不变。

Additional real socket REDs cover malformed HTTP fields and non-ASCII whitespace erasure. The initial Cursor fixture compilation error is preserved separately and does not count as behavioral RED. After using the actual TCP reader API, target failures were confirmed. Final results: seven exact cases,28 route observations,11 layout cases, and strict MCP package Clippy with no-deps passed. The42 dependency warnings and full-workspace/platform gates remain open.

最终候选保留合法URI reg-name（包括下划线与尾随点）的显式允许列表语义；不以额外DNS标签规则收窄已支持配置。registered_name_final保存同源码七项实际通过与原二进制/源码摘要，registered-name-clippy为最终包检查。
Final candidate preserves exact allowlisting of valid URI registered host names, including underscores and a trailing dot. No additional DNS label policy narrows supported configurations. The registered_name_final receipt/logs and final package Clippy record cover this candidate.
