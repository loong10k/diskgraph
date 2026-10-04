use crate::{
    DecodedTree, FlatNodes, Frame, FrameReader, ProtocolLimits, TreeWriter, tree_state::TreeState,
};
use diskgraph_disktree_core::tree::Node;
use std::io::{self, Read, Write};

/// 可信本机树的兼容写入工具；参数 root/output 为已拥有的扫描结果与流。
/// 返回节点数；产品调用应选显式原额度的 write_tree_with_limits，不能把此工具当资源验收。
/// 参数：output 为已有写入源，root 为调用者已拥有的可信树。
/// 返回：实际节点数或错误；这是无额外调用者额度的可信便利入口。
pub fn write_tree(output: &mut impl Write, root: &Node) -> io::Result<u64> {
    write_tree_with_limits(
        output,
        root,
        ProtocolLimits {
            max_frame_bytes: u64::from(u32::MAX),
            max_stream_bytes: u64::MAX,
            max_nodes: u64::MAX,
            max_depth: u64::MAX,
        },
    )
}

/// 用同一原账本迭代写出树及 End。参数 limits 必须由父请求提供；返回已发出的节点数。
/// 参数：output/root 为真实扫描输出与树，limits 为父请求原始配额。
/// 返回：完整 End 已写出的节点数或错误，不代表 OS wait。
pub fn write_tree_with_limits(
    output: &mut impl Write,
    root: &Node,
    limits: ProtocolLimits,
) -> io::Result<u64> {
    let mut writer = TreeWriter::new(output, limits);
    let mut count = 0_u64;
    let mut records = FlatNodes::new(root);
    while let Some(record) = records.next_with_limits(limits, writer.remaining_body_bytes()?) {
        writer.write_frame(&Frame::Node { node: record? })?;
        count = count
            .checked_add(1)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "node count overflow"))?;
    }
    writer.write_frame(&Frame::End { nodes: count })?;
    writer.finish()
}

/// 有界重建结果流并检查 End 后 EOF。返回可安全迭代销毁的树，不授予发布或 OS 退出许可。
/// 参数 limits 与 input 共享原始传输额度；错误阶段仅持有无子树节点，清理不会递归。
/// 参数：input 为阻塞结果流，limits 为同次节点/深度/原始字节上界。
/// 返回：通过 End 和 EOF 检查的迭代销毁 owner 或错误；非 OS 退出许可。
pub fn read_tree(input: impl Read, limits: ProtocolLimits) -> io::Result<DecodedTree> {
    let mut frames = FrameReader::new(input, limits);
    let mut state = TreeState::new(limits);
    let mut nodes = Vec::new();
    loop {
        let frame = frames
            .read_frame()?
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "missing End"))?;
        state.accept(&frame)?;
        match frame {
            Frame::Node { node } => {
                let parent = node.parent;
                let mut native = node.into_native()?;
                // 此处不按不可信 child_count 分配；在全结构验完后按实际边数预留。
                native.children.clear();
                nodes
                    .try_reserve(1)
                    .map_err(|_| io::Error::other("node allocation failed"))?;
                nodes.push((parent, native));
            }
            Frame::End { .. } => {
                frames.expect_eof()?;
                break;
            }
            _ => {}
        }
    }
    let mut result = DecodedTree::prepare(nodes.len())?;
    // 在形成递归所有权之前完成全部可能失败的 child 容量分配。
    let mut counts = Vec::new();
    counts
        .try_reserve_exact(nodes.len())
        .map_err(|_| io::Error::other("child count allocation failed"))?;
    counts.resize(nodes.len(), 0_usize);
    for (parent, _) in &nodes {
        if let Some(parent) = parent {
            let parent = usize::try_from(*parent)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "parent overflow"))?;
            counts[parent] = counts[parent].checked_add(1).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "child count overflow")
            })?;
        }
    }
    for ((_, node), count) in nodes.iter_mut().zip(counts) {
        node.children
            .try_reserve_exact(count)
            .map_err(|_| io::Error::other("children allocation failed"))?;
    }
    while nodes.len() > 1 {
        let (parent, mut child) = nodes.pop().expect("validated nonempty");
        child.children.reverse();
        nodes[parent.expect("validated parent") as usize]
            .1
            .children
            .push(child);
    }
    let (_, mut root) = nodes.pop().expect("validated root");
    root.children.reverse();
    result.install(root);
    Ok(result)
}
