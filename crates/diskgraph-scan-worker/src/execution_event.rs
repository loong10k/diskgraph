use crate::ScanProgress;

/// 一次完整 v2 帧的阶段观察；来源：PF-06，终态事件只指导关闭控制输入，不证明 EOF/OS 退出。
#[derive(Debug)]
pub enum ExecutionEvent {
    Hello,
    Progress(ScanProgress),
    Node { nodes: u64 },
    End { nodes: u64 },
    Failed,
}
