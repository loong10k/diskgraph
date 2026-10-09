# 本地提交撤权事实的末段预算优先级

真实SQLite内存数据库没有原生通知身份。原连接撤权并恢复授权后，末段SQL到期原实现丢失已知变化，返回BudgetExceeded。行为红灯2通过/1失败；最初Locator夹具格式错误单独保留，不作为行为红灯。

实现仅记录原事务提交成功的负向代次下界，弱绑定原连接。纯内存观察可保留Conflict；精确PermissionDenied仍优先。没有变化、无关写入、提交回滚、替换连接不会补造授权变化，绝不缓存Allow或延长期限。

Engine定向5通过；Store全targets 404通过、12忽略；Engine结构6通过；fmt与Clippy通过。日志和源码摘要见receipt。此证据不证明完整Engine测试、平台CI、扫描负载或生产验收通过。
