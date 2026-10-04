use crate::{FlatNode, Frame, ProtocolLimits, node_tags};
use std::io;
use std::path::{Component, Path};

/// 单方向结果流结构状态；来源：PF-06 有序平铺 Node，只有 End 可结束树阶段。
pub(crate) struct TreeState {
    limits: ProtocolLimits,
    count: u64,
    outstanding: u64,
    ancestors: Vec<(u64, u64)>,
    ended: bool,
}

impl TreeState {
    /// 参数：limits 为原结果流上界。
    /// 返回：零节点、尚未结束的先序结构状态。
    pub(crate) fn new(limits: ProtocolLimits) -> Self {
        Self {
            limits,
            count: 0,
            outstanding: 0,
            ancestors: Vec::new(),
            ended: false,
        }
    }
    /// 参数：self 为当前结构状态。
    /// 返回：已接受的真实节点数，不包含进度或 End。
    pub(crate) fn count(&self) -> u64 {
        self.count
    }
    /// 参数：self 为当前结构状态。
    /// 返回：是否已经接受完整且计数匹配的 End。
    pub(crate) fn ended(&self) -> bool {
        self.ended
    }

    /// 参数：frame 为借用的下一结果帧。
    /// 返回：通过后更新原计数/祖先状态；非法阶段、结构或额度返回错误。
    pub(crate) fn accept(&mut self, frame: &Frame) -> io::Result<()> {
        if self.ended {
            return Err(invalid("frame after End"));
        }
        match frame {
            Frame::Node { node } => self.node(node),
            Frame::Progress { .. } => Ok(()),
            Frame::End { nodes } => {
                if self.count == 0 || *nodes != self.count || !self.ancestors.is_empty() {
                    return Err(invalid("incomplete tree or End count"));
                }
                self.ended = true;
                Ok(())
            }
            Frame::Error { .. } => Err(invalid("worker reported scan failure")),
            _ => Err(invalid("unexpected result phase frame")),
        }
    }

    fn node(&mut self, node: &FlatNode) -> io::Result<()> {
        let next = self
            .count
            .checked_add(1)
            .filter(|next| *next <= self.limits.max_nodes)
            .ok_or_else(|| invalid("node count limit"))?;
        if node.sequence != self.count || node.depth > self.limits.max_depth {
            return Err(invalid("sequence or depth limit"));
        }
        let kind = node_tags::kind(node.kind)?;
        node_tags::category(node.category)?;
        if let Some(tag) = node.reclaim {
            node_tags::reclaim(tag)?;
        }
        if !kind.is_dir() && node.child_count != 0 {
            return Err(invalid("leaf owns children"));
        }
        if node.name.is_empty() || node.name.contains('\0') {
            return Err(invalid("invalid node name"));
        }
        if self.count == 0 {
            // root.name 是 pinned Node 的显示字段，可以含路径分隔符；实际根另外用 NativePath。
            if node.parent.is_some() || node.depth != 0 {
                return Err(invalid("invalid root structure"));
            }
        } else {
            validate_component(&node.name)?;
            let depth = self.ancestors.len() as u64;
            let Some((parent, remaining)) = self.ancestors.last_mut() else {
                return Err(invalid("extra root or child"));
            };
            if node.parent != Some(*parent) || node.depth != depth || *remaining == 0 {
                return Err(invalid("wrong parent or depth"));
            }
            *remaining -= 1;
        }
        // 声明的最低节点需求采用 checked arithmetic，巨大 child_count 不触发预分配。
        let outstanding = if self.count == 0 {
            Some(node.child_count)
        } else {
            self.outstanding
                .checked_sub(1)
                .and_then(|left| left.checked_add(node.child_count))
        }
        .ok_or_else(|| invalid("declared children overflow"))?;
        next.checked_add(outstanding)
            .filter(|total| *total <= self.limits.max_nodes)
            .ok_or_else(|| invalid("declared children exceed node limit"))?;
        self.outstanding = outstanding;
        if node.child_count != 0 {
            self.ancestors
                .try_reserve(1)
                .map_err(|_| io::Error::other("depth allocation failed"))?;
            self.ancestors.push((node.sequence, node.child_count));
        }
        while self.ancestors.last().is_some_and(|(_, left)| *left == 0) {
            self.ancestors.pop();
        }
        self.count = next;
        Ok(())
    }
}

fn validate_component(name: &str) -> io::Result<()> {
    let mut parts = Path::new(name).components();
    if !matches!(parts.next(), Some(Component::Normal(_)))
        || parts.next().is_some()
        || name.contains('/')
        || (cfg!(windows) && (name.contains('\\') || name.contains(':')))
    {
        return Err(invalid("invalid child component"));
    }
    Ok(())
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
