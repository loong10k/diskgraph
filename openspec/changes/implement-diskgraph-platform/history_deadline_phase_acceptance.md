# 历史数据到期与迟到能力验收分离

Linux MSRV 作业 112792548854（源码 ccf9c358，运行 37621476545）完整 Test 失败：scope 不兼容增长、scope 不兼容 changes、未知大小增长和历史比较前缀四个测试均在终检授权回调中等待原数据期限耗尽。新独立 50 ms 能力窗口会拒绝这些迟到允许，返回精确 Business(BudgetExceeded)。历史隔离测试成功不能替代此失败。

本轮不修改生产授权窗口或数据期限。四个夹具分别使用原 30 秒数据期限、终检第 3 次能力回调等待 80 ms，并精确拒为 Business(BudgetExceeded)，不接受任意错误或晚到前缀。

数据到期另由仅测试编译的真实读后同步点覆盖，使用及时的原策略授权器和合法 Store 发布元数据：

- 公开 compare API 保留一个实际文件行和 same=1，编码 complete=false、summary_is_partial=true、truncation_reason=deadline。
- 公开 growth API 分别覆盖未知大小和两个实际注册 scope。到期前严格返回 None；读后耗尽原期限后严格返回 Business(Timeout)，不能由不兼容/未知大小隐藏到期。

本机隔离元数据测试 2 passed（增长测试含两个场景）；scope 迟到能力测试 2 passed。其余两项原生集成夹具本机构造时返回 Unsupported，未执行功能断言，仍需新源码原生 CI。
增加同步点前，新测试因读后边界未执行而失败；这仅证明测试插桩尚未接入，不声称生产前缀功能此前缺失。
两路审查曾因增长 Timeout 覆盖缺失阻断，本轮已补充；补充后两路最终复核均 CLEAR/APPROVE。Engine source_layout 6 passed、Clippy all-target 和 CI 同范围 fmt 检查通过；新版本完整平台门禁仍未完成，生产任务保持开放。
