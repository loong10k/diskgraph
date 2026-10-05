/// 真实 stdin 控制动作；来源：PF-06 执行 v2，只有已见非 finished 实际进度后才发送。
#[derive(Clone, Copy, Debug)]
pub(crate) enum ControlAction {
    Cancel,
    EndOfInput,
    InvalidControl,
}

impl ControlAction {
    /// 参数：self 为本次唯一动作。
    /// 返回：固定完整正文，EOF 动作不写正文；不包含暂停或测试远程字段。
    pub(crate) fn payload(self) -> Option<&'static [u8]> {
        match self {
            Self::Cancel => Some(br#"{"type":"cancel"}"#),
            Self::EndOfInput => None,
            Self::InvalidControl => Some(br#"{"type":"cancel","unexpected":true}"#),
        }
    }

    /// 参数：self 为真实控制动作。
    /// 返回：按当前 WorkerControl 原路径应观察的固定错误 code 和 IO kind。
    pub(crate) fn expected(self) -> (&'static str, &'static str) {
        match self {
            Self::Cancel => ("cancelled", "interrupted"),
            Self::EndOfInput => ("protocol", "unexpected_eof"),
            Self::InvalidControl => ("protocol", "invalid_data"),
        }
    }
}
