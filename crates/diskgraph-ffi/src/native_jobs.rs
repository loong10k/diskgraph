use crate::JobHandle;
use std::path::PathBuf;
use std::sync::Weak;

/// 会话持有的规范根目录与扫描句柄弱引用列表，不延长作业订阅者生命周期。
/// 来源：DiskGraph 原生 Rust NativeService 作业登记；无 Java 对应对象。
pub(crate) type NativeJobs = Vec<(PathBuf, Weak<JobHandle>)>;
