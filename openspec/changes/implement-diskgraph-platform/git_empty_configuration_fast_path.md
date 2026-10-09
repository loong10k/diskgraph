# 已捕获空 Git 配置的子进程优化

沿用本变更 EC-02/04。当前 Windows 原生诊断表明子进程执行/收集是主要采样成本之一。只对完整捕获为零字节的配置省去 `git config --file --no-includes --null --list`，空配置解析结果确定为空。保留既有配置捕获、私有副本写入、元数据额度、源身份/内容终检、原始期限/取消与非空配置的原生解析。空白、注释及任何非空字节不能采用该捷径，不使用手写 INI 解析。

验收：空字节不调用解析操作；非空与空白调用原生操作并保持字段覆盖顺序；空字节仍拒绝原取消或过期；原生失败、不支持字段与解析期间撤权不得变成成功。运行原生空配置正向控制及受影响 Git 回归。定向通过不关闭 Windows 默认并发完整回归或 200k/300 秒门禁。

macOS 首次回归 4/1，确切失败为已捕获空配置意外调用解析操作；实现捷径后新增 6/0（含真实 Git 空文件控制），配置策略 3/0、元数据 16/0、隔离 21/0、Git 语义 19/0。Engine 全目标严格 Clippy 通过。本次四个源码文件的定向 rustfmt 检查通过；全 workspace fmt 检查仍报告既有 vendored disktree 排版差异，未修改上游文件或摘要。

Windows 台式机保持原 15 秒预算、实际 worker 与当前源码 driver。原行为在定向测试真实 RED 0/1，候选 GREEN 6/0；配置策略 3/0，元数据 14/0（Unix 专有用例不计为 Windows 通过）。原回执、stdout/stderr、源码前后摘要及实际程序身份见 `docs/benchmarks/windows_empty_config_{red,green,policy,metadata}_6faba945_2026_10_09.json`。所有源码变更为 6faba945 之上的明确覆盖，不能把 HEAD 当作候选唯一身份。GREEN 后只调整测试模块声明的排序；后续 policy/metadata 回执绑定该最终模块摘要。

一次回归驱动组装错误在 Python 语法解析阶段退出 1，未启动 Cargo，不计为行为 RED；随后以单目标驱动取得上述两个原生受影响回归结果。没有宣称默认并发完整 Engine、200k/300 秒、release 延迟或全部平台已通过。状态：本项实现与上述定向验证完成，生产父项保持开放。

2026-10-09 后续原生完整回归：相同台式机、默认测试并行度、原生产期限及相同 worker，在最终四个源码覆盖上运行完整 Engine 得到 710/19/7（通过/失败/忽略），214.37 秒；实际命令耗时 215.078 秒。源码前后摘要一致，driver 为 `1c19b5ec185414dc63b2ece43d740df8eba69aa6effe4be5a393f7d153ba458b`。回执与十九项失败摘录见 `windows_engine_after_empty_config_{receipt,failures}_6faba945_2026_10_09.json`。十八项在采样期限前不能完成或未到达要求的发布边界，另一项 32k 空文件创建在第 31677 项超过原 120 秒；仍有保留清理 owner 的诊断，不能当作恢复完成。此前完整回归 660/63/7 是另一轮观察，未经交错对照，失败数变化不单独证明因果或性能收益。

同期原提交 `6faba945` 的 GitHub Windows stable 作业 `113620149776` 已成功，包含 Engine 723/0/7、CLI 88/0/0 和 MCP library 231/0/0。其源码未包含本优化，与台式机原生失败必须并列保留；不同机器结果不能互相替代。本轮台式机 CLI 仍为 82/6/0，因 Cargo 提前停止，组合命令未执行 MCP；独立 MCP library 回归为 230/1/0，失败项 `encoded_socket_explain_expiry_preserves_bounded_partial_diagnostic` 在真实编码后返回 `budget_exceeded`，没有交付预期的部分诊断。另行运行 binary 与全部 integration targets，共 67/0/0，74.5 秒，包含真实 socket 的原令牌到期、数据库授权交集、跨主体隔离、Git 状态与断线后查询；两个零测试目标不能计为 Windows 平台能力通过。CLI、MCP library 与 integration 的独立原回执分别见 `windows_{cli,mcp,mcp_integrations}_after_empty_config_receipt_6faba945_2026_10_09.json`。生产父项继续开放。
