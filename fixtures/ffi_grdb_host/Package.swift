// swift-tools-version: 6.1
import Foundation
import PackageDescription

// 测试从当前工作树生成绑定，并显式定位同一次构建的 Rust 动态库。
guard let libraryDirectory = ProcessInfo.processInfo.environment["DISKGRAPH_RUST_LIBRARY_DIR"] else {
    fatalError("Run scripts/ffi-grdb-smoke.sh to generate bindings and locate the Rust library")
}
let package = Package(
    name: "DiskGraphGrdbHost",
    platforms: [.macOS(.v13)],
    dependencies: [
        .package(url: "https://github.com/groue/GRDB.swift.git", exact: "7.11.1")
    ],
    targets: [
        .systemLibrary(name: "diskgraph_ffiFFI", path: "bindings/ffi"),
        .target(
            name: "DiskGraphNative", dependencies: ["diskgraph_ffiFFI"],
            path: "bindings/swift",
            linkerSettings: [.linkedLibrary("diskgraph_ffi"), .unsafeFlags(["-L", libraryDirectory])]
        ),
        .executableTarget(
            name: "FfiGrdbHost",
            dependencies: ["DiskGraphNative", .product(name: "GRDB", package: "GRDB.swift")]
        )
    ]
)
