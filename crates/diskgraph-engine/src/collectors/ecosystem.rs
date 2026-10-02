//! 保存清单、同目录产物、重建工具及布局能否证明可重建的确定性规则。

/// 保存清单、同目录产物、重建工具及布局能否证明可重建的确定性规则。
/// 来源：原生 Rust diskgraph-engine::collectors::Ecosystem。
/// One ecosystem rule: which manifest declares the project, which sibling
/// directory is build output, which tool rebuilds it, and whether the layout
/// alone proves the output is rebuildable.
pub(super) struct Ecosystem {
    pub(super) manifest: &'static str,
    pub(super) output: &'static str,
    pub(super) tool: &'static str,
    /// Whether the deterministic layout justifies a `rebuildable_by` edge.
    /// Node's `node_modules` depends on a lockfile this collector does not
    /// observe, so it is owned but not claimed rebuildable.
    pub(super) proves_rebuildable: bool,
}
const ECOSYSTEMS: &[Ecosystem] = &[
    Ecosystem {
        manifest: "Cargo.toml",
        output: "target",
        tool: "cargo",
        proves_rebuildable: true,
    },
    Ecosystem {
        manifest: "package.json",
        output: "node_modules",
        tool: "npm",
        proves_rebuildable: false,
    },
    Ecosystem {
        // Maven and Gradle both keep build output in `target`; whichever
        // manifest sits beside it defines the project.
        manifest: "pom.xml",
        output: "target",
        tool: "maven",
        proves_rebuildable: true,
    },
    Ecosystem {
        manifest: "build.gradle",
        output: "build",
        tool: "gradle",
        proves_rebuildable: true,
    },
    Ecosystem {
        manifest: "build.gradle.kts",
        output: "build",
        tool: "gradle",
        proves_rebuildable: true,
    },
];
/// 查找清单名对应的确定性规则。
/// 参数：manifest 为清单文件名。
/// 返回：可选静态规则，不按名称猜测未知生态。
/// The ecosystem owning a manifest file name, if any.
pub(super) fn ecosystem_for_manifest(manifest: &str) -> Option<&'static Ecosystem> {
    ECOSYSTEMS.iter().find(|rule| rule.manifest == manifest)
}
