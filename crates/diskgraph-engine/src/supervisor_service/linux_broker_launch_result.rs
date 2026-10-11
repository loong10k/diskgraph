use crate::LinuxSupervisorProcess;
use crate::recovery_slot::LinuxSupervisorTrust;
use std::io;

/// 固定broker出生结果，失败必须保留已经出生的原监督owner以继续恢复。来源：PF-06，无Java对等对象。
pub(super) type LinuxBrokerLaunchResult = Result<
    (Box<LinuxSupervisorProcess>, LinuxSupervisorTrust),
    (io::Error, Option<Box<LinuxSupervisorProcess>>),
>;
