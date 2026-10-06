//! 集成与单元测试复用同一个真实宿主实现；来源：PF-06 原恢复责任夹具。
use diskgraph_engine::{
    Engine, EngineConfig, EngineError, ScanWorkerRecovery, ScanWorkerRuntimeBudget,
    ScanWorkerSettings,
};
#[path = "../../src/native_scan_engine_fixture.rs"]
mod native_scan_engine_fixture;
pub(crate) use native_scan_engine_fixture::NativeScanEngine;

#[cfg(windows)]
use diskgraph_engine::{ProbeHost, ProbeRecovery};
