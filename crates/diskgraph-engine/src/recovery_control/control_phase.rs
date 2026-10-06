/// 当前监督通知的有限阶段；来源：PF-06，无 Java 对等对象。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ControlPhase {
    AwaitReady,
    Running,
    ForegroundEnded,
    Recovering,
    Complete,
}
