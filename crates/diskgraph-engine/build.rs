fn main() {
    // 使用 Cargo 的真实编译目标对照 helper Hello；此值只校验协议，不建立镜像信任。
    let target = std::env::var("TARGET").expect("Cargo must provide TARGET");
    println!("cargo:rustc-env=DISKGRAPH_ENGINE_TARGET={target}");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let target_endian = std::env::var("CARGO_CFG_TARGET_ENDIAN").unwrap_or_default();
    println!("cargo:rerun-if-changed=src/native_child/linux_atomic_spawn.c");
    println!("cargo:rerun-if-changed=src/native_child/linux_atomic_syscall.h");
    println!("cargo:rerun-if-changed=src/macos_installation_metadata.c");
    println!("cargo:rerun-if-changed=src/native_child/macos_native_spawn.c");
    if target_os == "macos" {
        // 由 Apple SDK 处理 stat/ACL 的实际 ABI；Rust 只接收显式长度的卷身份字节。
        cc::Build::new()
            .file("src/macos_installation_metadata.c")
            .file("src/native_child/macos_native_spawn.c")
            .flag("-std=c11")
            .warnings_into_errors(true)
            .compile("diskgraph_macos_installation_metadata");
    }
    if target_os == "linux"
        && target_endian == "little"
        && matches!(target_arch.as_str(), "x86_64" | "aarch64")
    {
        // 子分支不允许编译器插入 TLS/运行库调用，关闭 sanitizer/protector/profile 注入。
        cc::Build::new()
            .file("src/native_child/linux_atomic_spawn.c")
            .opt_level(2)
            .flag("-std=c11")
            .flag("-ffreestanding")
            .flag("-fno-stack-protector")
            .flag("-fno-builtin")
            .flag("-fno-sanitize=all")
            .flag("-fno-profile-arcs")
            .warnings_into_errors(true)
            .compile("diskgraph_linux_atomic_spawn");
    }
}
