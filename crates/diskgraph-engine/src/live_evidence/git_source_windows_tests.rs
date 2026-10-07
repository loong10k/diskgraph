//! Windows 原句柄二阶段检查期间的确定性祖先活动／叶变化回归。
use super::root;
use crate::live_evidence::ProbeLimits;
use crate::live_evidence::probe_budget::ProbeBudget;
use std::cell::RefCell;
use std::ffi::OsStr;

type AttributeHook = Box<dyn FnMut(&OsStr, bool)>;
thread_local! {
    static HOOK: RefCell<Option<AttributeHook>> = const { RefCell::new(None) };
}
/// 参数：当前单组件名及祖先类别；返回：无，仅执行本测试线程已安装的确定性变更。
pub(super) fn after_attributes(name: &OsStr, ancestor: bool) {
    HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().as_mut() {
            hook(name, ancestor);
        }
    });
}
/// 当前线程夹具钩子的退场清理，防止失败后影响后续测试。
struct HookGuard;
impl Drop for HookGuard {
    fn drop(&mut self) {
        HOOK.with(|slot| *slot.borrow_mut() = None);
    }
}
fn changed_during_open(ancestor: bool) -> Result<(), String> {
    let holder = tempfile::tempdir().unwrap();
    let parent = holder.path().canonicalize().unwrap();
    let leaf = parent.join("registered-leaf");
    std::fs::create_dir(&leaf).unwrap();
    let target = if ancestor {
        parent.clone()
    } else {
        leaf.clone()
    };
    let name = target.file_name().unwrap().to_os_string();
    let changed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed = changed.clone();
    let version_mask = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let recorded_mask = version_mask.clone();
    HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move |component, route| {
            if component == name
                && route == ancestor
                && !observed.swap(true, std::sync::atomic::Ordering::Relaxed)
            {
                use std::os::windows::fs::OpenOptionsExt;
                use windows_sys::Win32::Storage::FileSystem::{
                    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_NO_RECALL,
                    FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
                };
                let attributes = std::fs::OpenOptions::new()
                    .access_mode(FILE_READ_ATTRIBUTES)
                    .custom_flags(
                        FILE_FLAG_BACKUP_SEMANTICS
                            | FILE_FLAG_OPEN_REPARSE_POINT
                            | FILE_FLAG_OPEN_NO_RECALL,
                    )
                    .open(&target)
                    .unwrap();
                let before = super::state(&attributes, true).unwrap();
                std::fs::create_dir(target.join("unrelated-entry")).unwrap();
                let after = super::state(&attributes, true).unwrap();
                // 先记录真实版本差异，允许原产品路径完成；末段仍强制核验夹具变更。
                recorded_mask.store(
                    before.changed_mask(&after),
                    std::sync::atomic::Ordering::Relaxed,
                );
            }
        }))
    });
    let _guard = HookGuard;
    let mut budget = ProbeBudget::new(&ProbeLimits::default()).unwrap();
    let result = root(&leaf, &mut budget).map(|_| ());
    assert!(
        changed.load(std::sync::atomic::Ordering::Relaxed),
        "actual source attributes boundary not exercised"
    );
    assert_ne!(
        version_mask.load(std::sync::atomic::Ordering::Relaxed) & 0x030,
        0,
        "actual directory time change not observed: ancestor={ancestor} product_result={result:?}"
    );
    result
}
#[test]
fn ancestor_sibling_change_between_attributes_and_data_keeps_original_route() {
    changed_during_open(true).expect("unrelated ancestor activity must preserve original route");
}
#[test]
fn registered_leaf_change_between_attributes_and_data_is_still_refused() {
    let error = changed_during_open(false).unwrap_err();
    assert!(
        error.contains("scoped Git source changed before data access"),
        "{error}"
    );
}
