# Windows 受控删除原语 / Controlled removal primitive

源码f75fe0eadc8c8d373a164f0a76c0612f501848c0，CI37430274941终态success。80个精确原生用例逐案实际1 passed / 0 failed / 0 ignored；原pending5、最后关闭后的原删除确认、游标不跳过原项、陌生根保留断言均通过。原先77/79的两个失败现在通过删除前订阅的原身份通知完成，没有将参数错误87改判为缺失。新增坏页及高位丢失回归通过。

`original_output.tar.gz`保存原始receipt、构建stdout/stderr及全部逐案输出。verification.json记录源码、冻结归档、fixture及原始输出摘要；逐案精确标记和原归档摘要均已核对。隔离候选仍基于原workspace8878912，未作为整个当前workspace通过。

仅完成原语的80项原生门禁，不完成生产递归owner/Pool、硬链接竞态、记录丢失后的恢复、有限退出、物理空间释放或其他平台门禁。危险工具仍关闭，生产父项不勾选。

All 80 exact native cases passed on source f75fe0e in run 37430274941, with one actual pass and no failures or ignored cases per process. Original pending, final-close, cursor-retention and foreign-root assertions passed. Error 87 is not reclassified as absence: confirmation uses the original prearmed identity notification. Bad-page and identity-loss tests passed. The archive retains the complete original receipt and output. This is isolated primitive qualification, not full current-workspace or production owner/Pool acceptance. Hard-link races, lost-record recovery, finite shutdown and the remaining platform gates are still open.
