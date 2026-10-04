use crate::process_evidence_codec::{field, object};
use crate::{
    ProcessEvidenceJobInput, ProcessObservationCode, ProcessObservationCoverage,
    ProcessObservationMethod, ProcessStartupIdentity,
};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
/// 逐资源采样的有限正向观察与覆盖摘要；来源：原生 Rust D42 / EV-06，不声明空集合可删除。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProcessEvidenceSummary {
    method: ProcessObservationMethod,
    capture_started_unix_ms: u64,
    capture_finished_unix_ms: u64,
    coverage: ProcessObservationCoverage,
    visibility_domain_sha256: [u8; 32],
    processes: Vec<ProcessStartupIdentity>,
    codes: Vec<ProcessObservationCode>,
}
impl ProcessEvidenceSummary {
    /// 参数：实际方法、窗口、覆盖、可见域、逐资源启动身份及固定缺失码；返回：规范有界观察。
    pub fn new(
        method: ProcessObservationMethod,
        window: (u64, u64),
        coverage: ProcessObservationCoverage,
        visibility_domain_sha256: [u8; 32],
        mut processes: Vec<ProcessStartupIdentity>,
        mut codes: Vec<ProcessObservationCode>,
    ) -> Result<Self, &'static str> {
        if window.0 == 0
            || window.1 < window.0
            || window.1 > i64::MAX as u64
            || visibility_domain_sha256.iter().all(|b| *b == 0)
            || processes.len() > 256
            || codes.len() > 9
        {
            return Err("invalid process evidence summary");
        }
        for p in &processes {
            p.validate()?;
            if !p.matches_method(method)
                || p.visibility_domain_sha256() != &visibility_domain_sha256
            {
                return Err("process observation domain mismatch");
            }
        }
        processes.sort();
        if processes.windows(2).any(|p| p[0] == p[1]) {
            return Err("duplicate process startup identity");
        }
        codes.sort();
        codes.dedup();
        if matches!(
            coverage,
            ProcessObservationCoverage::VisibleMethodDomainComplete
        ) != codes.is_empty()
        {
            return Err("invalid process coverage codes");
        }
        Ok(Self {
            method,
            capture_started_unix_ms: window.0,
            capture_finished_unix_ms: window.1,
            coverage,
            visibility_domain_sha256,
            processes,
            codes,
        })
    }
    /// 参数：无；返回：已固定的观察方法。
    pub fn method(&self) -> ProcessObservationMethod {
        self.method
    }
    /// 参数：无；返回：首次准备至末次观察的实际窗口。
    pub fn capture_window(&self) -> (u64, u64) {
        (self.capture_started_unix_ms, self.capture_finished_unix_ms)
    }
    /// 参数：无；返回：有限可见域覆盖，不代表全局无占用。
    pub fn coverage(&self) -> ProcessObservationCoverage {
        self.coverage
    }
    /// 参数：无；返回：该资源确实观察到的完整启动身份集合。
    pub fn processes(&self) -> &[ProcessStartupIdentity] {
        &self.processes
    }
    /// 参数：无；返回：固定原因码，不包含源路径或程序正文。
    pub fn codes(&self) -> &[ProcessObservationCode] {
        &self.codes
    }
    /// 参数：无；返回：由采样方法证明的实际可见域摘要。
    pub fn visibility_domain_sha256(&self) -> &[u8; 32] {
        &self.visibility_domain_sha256
    }
    /// 参数：原固定输入；返回：方法/结果摘要，不是文件正文或源字节指纹。
    pub fn observation_fingerprint(&self, input: &ProcessEvidenceJobInput) -> String {
        let mut h = Sha256::new();
        h.update(b"diskgraph-process-observation-v1\0");
        h.update(input.digest().as_bytes());
        h.update(serde_json::to_vec(self).expect("finite process summary serializes"));
        format!("{:x}", h.finalize())
    }
}
impl<'de> Deserialize<'de> for ProcessEvidenceSummary {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(deserializer)?;
        let o = object(
            &v,
            &[
                "method",
                "capture_started_unix_ms",
                "capture_finished_unix_ms",
                "coverage",
                "visibility_domain_sha256",
                "processes",
                "codes",
            ],
        )
        .map_err(serde::de::Error::custom)?;
        let result = Self::new(
            field(o, "method").map_err(serde::de::Error::custom)?,
            (
                field(o, "capture_started_unix_ms").map_err(serde::de::Error::custom)?,
                field(o, "capture_finished_unix_ms").map_err(serde::de::Error::custom)?,
            ),
            field(o, "coverage").map_err(serde::de::Error::custom)?,
            field(o, "visibility_domain_sha256").map_err(serde::de::Error::custom)?,
            field(o, "processes").map_err(serde::de::Error::custom)?,
            field(o, "codes").map_err(serde::de::Error::custom)?,
        )
        .map_err(serde::de::Error::custom)?;
        if serde_json::to_value(&result).map_err(serde::de::Error::custom)? != v {
            return Err(serde::de::Error::custom("noncanonical process observation"));
        }
        Ok(result)
    }
}
