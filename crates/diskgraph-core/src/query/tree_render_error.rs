/// 树展示失败原因；来源：DiskGraph 原生 Rust query::TreeRenderError。
/// Why a tree view could not be rendered.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TreeRenderError {
    NoRoot,
}

impl std::fmt::Display for TreeRenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoRoot => "the revision has no root node",
        })
    }
}
