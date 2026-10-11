use super::ChildError;
use std::io;

/// 由受信启动器固定选择的 Linux 启动角色，区分扫描过滤与监督服务降权阶段。
/// 来源：原生 Rust 原子启动与 Linux 监督服务固定消息协议。
#[derive(Clone, Copy)]
pub(super) enum LinuxStartupRole {
    Atomic,
    Supervisor,
}

impl LinuxStartupRole {
    /// 参数：phase 是原消息阶段；返回：该受信角色是否允许此失败阶段。
    pub(super) fn failure_phase(self, phase: u32) -> bool {
        (1..=11).contains(&phase) || matches!(self, Self::Supervisor) && (12..=18).contains(&phase)
    }

    /// 参数：kind、phase、error 为固定消息字段；返回：Ready 或保留原 errno 的启动失败。
    pub(super) fn verify(self, kind: u32, phase: u32, error: i32) -> Result<(), ChildError> {
        if kind == 1 && phase == 0 && error == 0 {
            return Ok(());
        }
        if kind == 2 && self.failure_phase(phase) && (1..=4095).contains(&error) {
            let context = match (self, phase) {
                (Self::Supervisor, 7) => "supervisor child capability setup",
                (Self::Supervisor, 8) => "supervisor child credentials",
                (Self::Supervisor, 12) => "supervisor child keepcaps",
                (Self::Supervisor, 13) => "supervisor child clear ambient",
                (Self::Supervisor, 14) => "supervisor child no_new_privs",
                (Self::Supervisor, 15) => "supervisor child setgroups",
                (Self::Supervisor, 16) => "supervisor child setresgid",
                (Self::Supervisor, 17) => "supervisor child setresuid",
                (Self::Supervisor, 18) => "supervisor child capset",
                (_, 1) => "atomic child dup3",
                (_, 2) => "atomic child close_range",
                (_, 3) => "atomic child reset handler",
                (_, 4) => "atomic child reset mask",
                (_, 5) => "atomic child Init",
                (_, 6) => "atomic child setsid",
                (_, 7) => "atomic child no_new_privs",
                (_, 8) => "atomic child seccomp",
                (_, 9) => "atomic child Ready",
                (_, 10) => "atomic child ACK",
                _ => "atomic child execveat",
            };
            return Err(ChildError::io(context, io::Error::from_raw_os_error(error)));
        }
        Err(ChildError::io(
            "atomic startup protocol",
            io::Error::new(io::ErrorKind::InvalidData, "unexpected startup message"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::LinuxStartupRole;

    #[test]
    fn supervisor_credential_failure_is_not_seccomp() {
        let error = LinuxStartupRole::Supervisor.verify(2, 8, 1).unwrap_err();
        assert!(
            error.to_string().contains("supervisor child credentials"),
            "{error}"
        );
        assert_eq!(error.native_io_error().unwrap().raw_os_error(), Some(1));
    }

    #[test]
    fn ordinary_seccomp_stage_remains_unchanged() {
        let error = LinuxStartupRole::Atomic.verify(2, 8, 1).unwrap_err();
        assert!(error.to_string().contains("atomic child seccomp"));
    }
    #[test]
    fn precise_supervisor_phases_preserve_errno_and_reject_role_confusion() {
        for (phase, operation) in [
            (12, "keepcaps"),
            (13, "clear ambient"),
            (14, "no_new_privs"),
            (15, "setgroups"),
            (16, "setresgid"),
            (17, "setresuid"),
            (18, "capset"),
        ] {
            let error = LinuxStartupRole::Supervisor
                .verify(2, phase, 1)
                .unwrap_err();
            assert!(error.to_string().contains(operation), "{error}");
            assert_eq!(error.native_io_error().unwrap().raw_os_error(), Some(1));
            let error = LinuxStartupRole::Atomic.verify(2, phase, 1).unwrap_err();
            assert_eq!(
                error.native_io_error().unwrap().kind(),
                std::io::ErrorKind::InvalidData
            );
        }
    }

    #[test]
    fn malformed_packets_do_not_become_ready_or_native_failure() {
        for role in [LinuxStartupRole::Atomic, LinuxStartupRole::Supervisor] {
            assert!(role.verify(1, 0, 0).is_ok());
            for (kind, phase, errno) in [
                (1, 15, 0),
                (1, 0, 1),
                (2, 0, 1),
                (2, 19, 1),
                (2, 8, 0),
                (2, 8, -1),
                (2, 8, 4096),
                (3, 0, 0),
            ] {
                let error = role.verify(kind, phase, errno).unwrap_err();
                assert_eq!(
                    error.native_io_error().unwrap().kind(),
                    std::io::ErrorKind::InvalidData
                );
            }
        }
    }
}
