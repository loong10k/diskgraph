//! 标准测试模块与既有 crate::tests 夹具路径。
mod authorization_tests;
mod candidate_tests;
mod envelope_tests;
mod management_tests;
mod protocol_tests;
mod query_tests;
mod relation_tests;
mod stdio_tests;
mod support;
pub(crate) use support::{call, cargo_project, seed, service};
