# diskgraph-testkit

Isolated filesystem fixtures for DiskGraph tests: sparse files, hard
links, symlink loops, unreadable directories, non-UTF-8 names, and the
manifest of scenarios that only a real operating system can validate.

## Install

```toml
[dev-dependencies]
diskgraph-testkit = "0.2"
```

## Usage

```rust,ignore
use diskgraph_testkit::FixtureTree;

let tree = FixtureTree::new("example").unwrap();
tree.dir("project/src").unwrap();
tree.file("project/src/main.rs", 1024).unwrap();
tree.sparse_file("project/target/sparse.bin", 1 << 20).unwrap();
```

## The real-OS manifest

`real_os_requirements()` returns the scenarios that unit fixtures must
not fake — cloud placeholders that must not hydrate, volume swaps under
an open path, per-platform trash backends, and mobile provider
lifecycles. CI compiles the drills that need real hosts and documents the
rest rather than pretending they passed.

## License

MIT
