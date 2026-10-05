//! 后验诊断及其真实持久查询验证；来源：原生 Rust CLI 集成测试。

mod reader;
mod tests;

pub(super) use reader::read;
