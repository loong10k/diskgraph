# SSE身份检查的有界控制观察

状态：实施中。适用现代与legacy SSE共用的identity_is_live。不得仅缓存权限或只检查token；原主体、token能力与实时数据库grant交集保持。每次身份轮询建立50ms单调绝对窗口，锁等待、策略与scope SQL及结果检查共用，不刷新。未完成或错误fail closed；任意同步OS调度不承诺硬实时。

持控制锁时身份观察须在独立owner仍持锁时返回false，不能依赖释放锁或socket shutdown才退出。随后覆盖真实现代/legacy连接竞争、EOF、主体槽释放与stop/join；现有正常心跳、到期、撤权继续通过。不取消已入队业务job，不把同步业务硬退出或监督恢复闭环列为本项完成。

当前证据：macOS持锁身份检查行为RED 0/1，候选1/0；legacy传输7/0；Engine/MCP all-target Clippy通过。真实现代/legacy SSE建连后持锁、EOF/原服务槽释放及shutdown/join已补：Linux旧实现0/3、候选3/3，macOS候选3/3。双路审查APPROVE/CLEAR；Windows及同提交CI待验收，未声明全平台门禁完成。
