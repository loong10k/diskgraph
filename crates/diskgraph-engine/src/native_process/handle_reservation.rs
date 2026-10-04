use std::cell::Cell;

/// 原生会话的借用额度释放器；来源：Rust D42，不拥有或关闭操作系统句柄。
/// 实际 File 等原生资源仍由各自 owner 释放，此对象只保证 unwind 不泄漏计数。
pub(super) struct HandleReservation<'a> {
    count: &'a Cell<u32>,
    reserved: u32,
}
impl<'a> HandleReservation<'a> {
    /// 参数：已成功增加的会话计数与本次额度；返回：仅借用同一计数的 RAII guard。
    pub(super) fn new(count: &'a Cell<u32>, reserved: u32) -> Self {
        Self { count, reserved }
    }
}
impl Drop for HandleReservation<'_> {
    fn drop(&mut self) {
        self.count.set(self.count.get() - self.reserved);
    }
}
