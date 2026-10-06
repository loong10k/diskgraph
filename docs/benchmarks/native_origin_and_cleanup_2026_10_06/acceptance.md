# 同源码原生专项复核 / Source-bound native qualification

源码 / Source: `2cc573c870dbad371113fc164530bab70efdb749`。

- MCP Origin CI37426844169：Linux与Windows各7个精确测试，每案实际1 passed / 0 failed / 0 ignored；含真实socket路由。macOS仍排队，不能宣布三平台完成。
- Linux1261个运行时源码哈希与本机逐字节匹配；Windows15个逐字节匹配、1246个与本机转换CRLF后的哈希匹配。保留原receipt，不把换行等价称为逐字节一致。
- Windows cleanup CI37426849148：77个实际原生案，75通过、2失败。新增删除前身份/预算门禁实际通过；最后外部句柄关闭后原ID证明仍返回87，保留原失败语义。
- `verification.json`记录逐案stdout复核、平台和fixture摘要，压缩文件保留原始内容。隔离原生候选不是产品集成验收，不勾选平台生产父项。

Linux and Windows each executed all seven exact Origin cases successfully, including real socket routes. macOS is still queued. Windows cleanup executed 77 native candidate cases: 75 passed, two final deletion witnesses failed with error 87. The new identity and pre-mutation budget guard passed. Original receipts, build logs and per-case output are retained. Runtime Windows source hashes match the local files after explicit CRLF conversion where needed; this is not byte-for-byte equivalence. Full product cleanup, owner integration and production acceptance remain incomplete.
