use crate::windows_file_state::WindowsFileState;
use std::cell::Cell;
use std::marker::PhantomData;
use std::rc::Rc;

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static PHASE: Cell<&'static str> = const { Cell::new("unknown") };
    static MISMATCH_PHASE: Cell<&'static str> = const { Cell::new("unknown") };
    static BRANCH: Cell<Option<&'static str>> = const { Cell::new(None) };
    static DIRECTORY: Cell<bool> = const { Cell::new(false) };
    static MASK: Cell<u16> = const { Cell::new(0) };
}

/// 请求局部 Windows 源状态诊断；来源：原生 Rust Git 源能力与 WindowsFileState。
/// 仅复用已捕获状态的差异位，不保存路径、文件 ID、时间值或正文；退场恢复原 TLS。
pub(super) struct GitSourceWindowsDiagnostic {
    previous_enabled: bool,
    previous_phase: &'static str,
    previous_mismatch_phase: &'static str,
    previous_branch: Option<&'static str>,
    previous_directory: bool,
    previous_mask: u16,
    _thread_bound: PhantomData<Rc<()>>,
}

impl GitSourceWindowsDiagnostic {
    /// 参数：无；仅目标测试在同一采样线程创建此守卫。
    /// 返回：已清空本次固定大小诊断并持有原 TLS 的请求守卫，不创建新预算。
    pub(super) fn new() -> Self {
        Self {
            previous_enabled: ENABLED.replace(true),
            previous_phase: PHASE.replace("unknown"),
            previous_mismatch_phase: MISMATCH_PHASE.replace("unknown"),
            previous_branch: BRANCH.replace(None),
            previous_directory: DIRECTORY.replace(false),
            previous_mask: MASK.replace(0),
            _thread_bound: PhantomData,
        }
    }

    /// 参数：initial/current 为同次原有状态，directory 为原类型参数，branch 为固定比较分支。
    /// 返回：无；仅记录本请求第一次实际不相等，不捕获状态、不格式化或输出。
    pub(super) fn record(
        initial: &WindowsFileState,
        current: &WindowsFileState,
        directory: bool,
        branch: &'static str,
    ) {
        if !ENABLED.get() || BRANCH.get().is_some() {
            return;
        }
        MISMATCH_PHASE.set(PHASE.get());
        DIRECTORY.set(directory);
        MASK.set(initial.changed_mask(current));
        BRANCH.set(Some(branch));
    }

    /// 参数：phase 为源码调用位置的固定标签，不能来自路径或工具输出。
    /// 返回：启用诊断时的旧阶段；未启用返回 None，不影响其它测试。
    pub(super) fn replace_phase(phase: &'static str) -> Option<&'static str> {
        ENABLED.get().then(|| PHASE.replace(phase))
    }

    /// 参数：phase 为同一请求阶段守卫保存的原标签。
    /// 返回：无；只恢复 TLS 标签，不改变采样状态、错误或时钟。
    pub(super) fn restore_phase(phase: &'static str) {
        PHASE.set(phase);
    }

    /// 参数：self 为仍在原线程的请求守卫；调用者必须已结束实际采样。
    /// 返回：无；输出固定阶段、分支、原类型和差异位，未见证的字段明确为 unknown。
    pub(super) fn report(&self) {
        let branch = BRANCH.get();
        let kind = match branch {
            Some(_) if DIRECTORY.get() => "directory",
            Some(_) => "file",
            None => "unknown",
        };
        eprintln!(
            "SCOPED_GIT_WINDOWS_CHILD_DIAGNOSTIC observed={} phase={} branch={} kind={kind} changed_mask={:#05x} mask_bits=volume:1,file_id:2,eof:4,creation:8,last_write:16,change_time:32,attributes:64,directory:128,delete_pending:256",
            branch.is_some(),
            MISMATCH_PHASE.get(),
            branch.unwrap_or("unknown"),
            MASK.get(),
        );
    }
}

impl Drop for GitSourceWindowsDiagnostic {
    fn drop(&mut self) {
        MASK.set(self.previous_mask);
        DIRECTORY.set(self.previous_directory);
        BRANCH.set(self.previous_branch);
        MISMATCH_PHASE.set(self.previous_mismatch_phase);
        PHASE.set(self.previous_phase);
        ENABLED.set(self.previous_enabled);
    }
}
