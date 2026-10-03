/// 树展示所需的窄节点，不携带原生定位和文件身份；来源：DiskGraph 原生 Rust query::TreeNode。
/// The fields a tree view actually renders. The full `DiskNode` carries
/// locators, hints, and identities that a tree never shows; building one
/// from this narrow shape skips all of that (the store's fast read path
/// passes these through without any JSON at all).
#[derive(Clone, Copy, Debug)]
pub struct TreeNode<'a> {
    pub id: u64,
    pub parent_id: Option<u64>,
    pub name: &'a str,
    pub kind: crate::NodeKind,
    pub subtree_bytes: u64,
    pub direct_bytes: u64,
    pub files: u64,
    pub directories: u64,
    pub read_error: bool,
    /// The classification a collector assigned; a view colours by it.
    pub category_hint: Option<&'a str>,
}
