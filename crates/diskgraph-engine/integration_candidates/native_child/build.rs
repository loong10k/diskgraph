fn main() {
    // 使用 Cargo 的真实编译目标对照 helper Hello；此值只校验协议，不建立镜像信任。
    let target = std::env::var("TARGET").expect("Cargo must provide TARGET");
    println!("cargo:rustc-env=DISKGRAPH_ENGINE_TARGET={target}");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let target_endian = std::env::var("CARGO_CFG_TARGET_ENDIAN").unwrap_or_default();
    println!("cargo:rerun-if-changed=src/native_child/linux_atomic_spawn.c");
    println!("cargo:rerun-if-changed=src/native_child/linux_atomic_syscall.h");
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
