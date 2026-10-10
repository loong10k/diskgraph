# MCP 兼容退出的原期限轮询

沿用PF-06及既有原owner恢复合同。MCP兼容等待路径与CLI采用已实现的原池drain_until；不调用同步cleanup/wait。每轮取得50ms绝对恢复窗口，双池共享该窗口，不刷新业务请求期限。关闭全部准入后，过期返回Pending；同轮分别尝试原扫描和Windows探针，扫描错误不阻止探针尝试，原错误仍优先传播。完成确认也须在本轮期限内；Pending/错误保留原恢复责任。操作系统单调用仍不承诺硬墙钟上限。

新增真实隔离数据库回归：原窗口过期但原池为空，旧同步drain实际返回true，RED在expired recovery observation must remain unconfirmed断言失败。改为drain_until后GREEN，并沿原finish路径验证幸存Engine不能再出生扫描、无revision发布。镜像仅三字节构造夹具，未执行，不作为真实scanner证明。

本机macOS MCP binary全部7项通过，包含本回归、容量和原service clone退休；MCP all-targets Clippy -D warnings、格式和diff检查通过。Windows条件路径与完整同源码workspace仍需CI。本变更不关闭无限外层兼容等待、独立监督角色、有限公共EOF或生产门禁；不启用写操作。
