//! vm_caveat：既有文件操作职责的原生 Rust 实现。

/// The note that bounds every byte number Docker reports on a desktop VM.
pub const VM_CAVEAT: &str = "Docker object bytes live inside the Docker VM disk; \
     freeing them does not immediately shrink the host file, and the VM disk \
     image itself is never treated as a cleanable cache";
