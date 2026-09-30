## DiskGraph

A disk usage index for this project. A `.diskgraph/` directory at the project
root means the tree has been indexed; there is nothing to use if there is not
one.

**Reach for it before guessing about disk space.** `du -sh *`, a recursive
`find`, and a directory listing in a loop all cost more context and are less
accurate than one indexed query.

```bash
diskgraph du                     # one summary per top-level entry
diskgraph top --scope <id> -n 20 # the twenty largest directories
diskgraph tree --scope <id> --depth 2        # a JSON tree
diskgraph tree --scope <id> --html out.html  # the same tree as a picture
diskgraph tui --scope <id>      # interactive, terminal
diskgraph growth --before <rev> --after <rev> --path src/lib   # what changed
diskgraph changes --before <rev> --after <rev>  # what was added or removed
```

An agent with the DiskGraph MCP server available should prefer
`diskgraph_top` and `diskgraph_children`; they answer the same questions
without a shell.

**Size is not the whole question.** When asked what to clean up, report what
is large *and* what is rebuildable — a cache is large and disposable, a
source tree is large and not. Do not turn a size report into a deletion
recommendation the user did not ask for.

**If there is no `.diskgraph/` directory, skip DiskGraph entirely** and use
the ordinary tools. Indexing is the user's decision, not yours.
