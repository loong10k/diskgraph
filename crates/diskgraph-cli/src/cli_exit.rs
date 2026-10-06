use std::process::{ExitCode, ExitStatus};

/// CLI 工作线程完成后的退出结果；来源：原生 Rust 服务转交合同，无 Java 对等对象。
/// 伴随进程保留原 ExitStatus，避免 Windows 32 位错误码被截断为成功。
pub(crate) enum CliExit {
    /// 普通命令已完成全部宿主恢复后的标准业务退出码。
    Code(ExitCode),
    /// serve 未建立 CLI 引擎；真实伴随进程已经 wait 完成的原退出状态。
    Companion(ExitStatus),
}
