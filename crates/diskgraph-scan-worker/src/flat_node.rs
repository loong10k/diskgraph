use crate::node_tags;
use diskgraph_disktree_core::tree::Node;
use serde::{Deserialize, Serialize};
use std::io;

/// 上游 Node 的非递归线格式；来源：pinned tree.rs，children 改为有序先序记录。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlatNode {
    pub sequence: u64,
    pub parent: Option<u64>,
    pub depth: u64,
    pub child_count: u64,
    pub name: String,
    pub kind: u8,
    pub bytes: u64,
    pub own_bytes: u64,
    pub files: u64,
    pub own_files: u64,
    pub dirs: u64,
    pub inode: Option<(u64, u64)>,
    pub read_error: bool,
    pub modified: i64,
    pub category: u8,
    pub reclaim: Option<u8>,
}

impl FlatNode {
    /// 从借用节点建立单条记录；参数 sequence/parent/depth 为迭代遍历产生的结构位置。
    /// 返回保留全部原生字段、但不复制子树的记录。
    /// 参数：node 为借用 pinned 节点，sequence/parent/depth 为其先序位置。
    /// 返回：复制单节点全部字段的记录，不复制子树。
    pub fn from_native(node: &Node, sequence: u64, parent: Option<u64>, depth: u64) -> Self {
        Self {
            sequence,
            parent,
            depth,
            child_count: node.children.len() as u64,
            name: node.name.to_string(),
            kind: node_tags::kind_tag(node.kind),
            bytes: node.bytes,
            own_bytes: node.own_bytes,
            files: node.files,
            own_files: node.own_files,
            dirs: node.dirs,
            inode: node.inode,
            read_error: node.read_error,
            modified: node.modified,
            category: node_tags::category_tag(node.category),
            reclaim: node.reclaim.map(node_tags::reclaim_tag),
        }
    }

    /// 恢复不含子节点的原生 Node；返回所有字段或未知枚举标签错误。
    /// 参数：self 为单条平铺节点记录。
    /// 返回：空 children 的原生节点；未知类别、类型或 reclaim 标签返回错误。
    pub fn into_native(self) -> io::Result<Node> {
        Ok(Node {
            name: self.name.into_boxed_str(),
            kind: node_tags::kind(self.kind)?,
            bytes: self.bytes,
            own_bytes: self.own_bytes,
            files: self.files,
            own_files: self.own_files,
            dirs: self.dirs,
            inode: self.inode,
            read_error: self.read_error,
            modified: self.modified,
            category: node_tags::category(self.category)?,
            reclaim: self.reclaim.map(node_tags::reclaim).transpose()?,
            children: Vec::new(),
        })
    }
}
