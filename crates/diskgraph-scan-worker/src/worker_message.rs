use serde::{Serialize, Serializer};
use std::io;

/// IO 消息的借用序列化视图；来源：serde::Serializer::collect_str，避免先建立完整错误字符串。
pub(crate) struct WorkerMessage<'a>(pub(crate) &'a io::Error);

impl Serialize for WorkerMessage<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self.0)
    }
}
