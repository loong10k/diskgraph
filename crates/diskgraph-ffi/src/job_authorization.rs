/// 作业返回数据前重新核对持久实时权限的共享闭包。
/// 来源：DiskGraph 原生 Rust FFI 作业授权；无 Java 对应对象。
pub(crate) type JobAuthorization = std::sync::Arc<dyn Fn() -> Result<(), String> + Send + Sync>;
