fn main() {
    let target = std::env::var("TARGET").expect("Cargo supplies the actual compilation target");
    println!("cargo:rustc-env=DISKGRAPH_WORKER_TARGET={target}");
    println!("cargo:rerun-if-changed=build.rs");
}
