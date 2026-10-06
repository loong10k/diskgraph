# Windows 通知时机实验 / Notification timing experiment

CI37428870214，源码7995aa57703fa9f1fb372993444a6889df60db4a，79案实际执行，77通过、2失败。两个新增通知案各1 passed / 0 failed / 0 ignored，均观察并输出原身份标记。外部句柄保活的200ms负控无原REMOVE；最后关闭后同FileId/ParentFileId REMOVE正控成功。原名称移动的OLD/NEW及陌生同名创建/删除记录不匹配原身份；实际移除移动后的原对象才匹配。原ID查询的两个失败仍保留错误87，整个门禁为failure。

这里只验证通知时机与成员身份匹配，不证明物理空间立即释放、ReFS、产品递归walker、owner/Pool集成、通知溢出恢复或有限退出。测试Drop仍是保活内存的阻塞兜底。不能据此启用平台或勾选生产父项。

79 native cases actually ran: 77 passed and two original ID deletion witnesses failed with error 87. Both notification experiments passed, including the 200ms external-handle negative control and same-ID last-close positive control. Rename and foreign same-name removal did not match the original identity. This does not establish physical capacity release, ReFS support, production ownership integration, notification-loss recovery or finite shutdown. Original outputs and fixture/source receipts are retained.
