use super::ChildError;
use super::linux_atomic_child::LinuxAtomicChild;

/// 借用原出生后owner的请求局部观察回调，不建立第二个owner或启动许可。
/// 来源：原生 Rust LinuxAtomicLauncher 的既有回调契约。
pub(super) type LinuxAtomicBirthObserver<'a> =
    dyn FnMut(&mut LinuxAtomicChild) -> Result<(), ChildError> + 'a;
