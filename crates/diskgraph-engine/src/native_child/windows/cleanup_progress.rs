/// 非阻塞处置的真实观察结果；来源：PF-06有限清理，Pending不返还owner或活动容量。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CleanupProgress {
    /// 原期限耗尽或内核责任尚未完成，调用者必须继续持有同一owner。
    Pending,
    /// 所有原责任已获得真实完成观察，此后才允许释放对应owner。
    Complete,
}
