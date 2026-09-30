## DiskGraph

本项目的磁盘占用索引。项目根目录有 `.diskgraph/` 就说明已经建过索引；没有
就不要用 DiskGraph。

**在猜测磁盘占用之前先用它。** `du -sh *`、递归 `find`、循环列目录，
既更费上下文，答案也不如一次索引查询准。

```bash
diskgraph du                     # 顶层条目各自多大
diskgraph top --scope <id> -n 20 # 最大的二十个目录
diskgraph tree --scope <id> --depth 2        # JSON 目录树
diskgraph tree --scope <id> --html out.html  # 同一棵树，画成图
diskgraph tui --scope <id>      # 终端里交互式浏览
diskgraph growth --before <rev> --after <rev> --path src/lib   # 哪里长大了
diskgraph changes --before <rev> --after <rev>  # 增删了什么
```

如果智能体接了 DiskGraph 的 MCP，优先用 `diskgraph_top` 和
`diskgraph_children`——同样的问题，不用开 shell。

**体积不是全部。** 用户问"该清理什么"时，要同时说明什么大、什么可重建：
缓存大而且可以随时再生成，源码树大但不能。**不要把一份体积报告变成用户
没有要求的删除建议。**

**如果项目里没有 `.diskgraph/` 目录，就完全不要用 DiskGraph**，用普通工具
就好。建不建索引是用户的决定，不是你的。
