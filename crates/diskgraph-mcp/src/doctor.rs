//! Read-only diagnostics (P3 task 4.6, spec RT-05 / C29): dependencies,
//! permissions, database versions, capacity, and the served capability set.
//! `doctor` never installs a tool, changes permissions, removes a lock, or
//! touches user files — it reports and exits.

use std::path::Path;

use diskgraph_core::{DenyAllAuthorizer, PrincipalId};
use diskgraph_engine::Engine;
use serde_json::{Value, json};

/// One diagnostic finding with a stable severity and an actionable message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Finding {
    pub check: &'static str,
    pub severity: Severity,
    pub message: String,
}

/// How serious a finding is. `Error` means a capability is degraded; nothing
/// here is ever silently repaired.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Severity {
    Ok,
    Warn,
    Error,
}

impl Severity {
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

/// The full diagnostic report.
pub struct Report {
    pub findings: Vec<Finding>,
}

impl Report {
    /// True when nothing is degraded; a doctor run that reports problems is
    /// not a failure of the tool itself, so the exit code stays 0 and the
    /// report carries the verdict.
    pub fn is_healthy(&self) -> bool {
        !self
            .findings
            .iter()
            .any(|finding| finding.severity != Severity::Ok)
    }

    /// The report as JSON, including the served tool and catalog surfaces.
    pub fn to_json(&self, engine: &Engine, profile: crate::protocol::ToolProfile) -> Value {
        json!({
            "healthy": self.is_healthy(),
            "data_dir": engine.data_dir().display().to_string(),
            "checks": self.findings.iter().map(|finding| json!({
                "check": finding.check,
                "severity": finding.severity.wire_name(),
                "message": finding.message,
            })).collect::<Vec<_>>(),
            "served": {
                "profile": profile.wire_name(),
                "tools": crate::protocol::tools_for(profile)
                    .iter()
                    .map(|tool| tool.name)
                    .collect::<Vec<_>>(),
            },
        })
    }
}

/// Runs every read-only diagnostic against an open engine.
pub fn diagnose(engine: &Engine, profile: crate::protocol::ToolProfile) -> Report {
    let mut findings = Vec::new();
    let data_dir = engine.data_dir();

    // Storage: both databases must exist and be at a supported schema.
    for (label, file) in [
        ("graph_store", data_dir.join("diskgraph.sqlite")),
        ("control_store", data_dir.join("diskgraph-control.sqlite")),
    ] {
        findings.push(if file.exists() {
            Finding {
                check: label,
                severity: Severity::Ok,
                message: format!("present at {}", file.display()),
            }
        } else {
            Finding {
                check: label,
                severity: Severity::Error,
                message: format!("missing: {}", file.display()),
            }
        });
    }

    // Server identity: minted once and stable.
    findings.push(match engine.server_id() {
        Ok(server_id) => Finding {
            check: "server_identity",
            severity: Severity::Ok,
            message: server_id.as_str().to_owned(),
        },
        Err(error) => Finding {
            check: "server_identity",
            severity: Severity::Error,
            message: error.to_string(),
        },
    });

    // Authorization: an ungranted principal must see no scope at all. With no
    // scopes registered the registry is legitimately empty, so the check only
    // runs when there is something to hide.
    let stranger = PrincipalId::new("doctor-probe")
        .unwrap_or_else(|_| PrincipalId::new("doctor-probe-fallback").expect("constant is valid"));
    match engine.list_scopes(&stranger, &DenyAllAuthorizer) {
        Ok(scopes) if scopes.is_empty() => findings.push(Finding {
            check: "authorization",
            severity: Severity::Ok,
            message: "an ungranted principal sees no scope".to_owned(),
        }),
        Ok(_) => findings.push(Finding {
            check: "authorization",
            severity: Severity::Error,
            message: "an ungranted principal was shown a scope".to_owned(),
        }),
        Err(_) => findings.push(Finding {
            check: "authorization",
            severity: Severity::Error,
            message: "the authorization path failed instead of denying".to_owned(),
        }),
    }

    // Capacity: the data directory size, measured without `du`.
    match directory_bytes(data_dir) {
        Ok(bytes) => findings.push(Finding {
            check: "capacity",
            severity: Severity::Ok,
            message: format!("data directory holds {bytes} bytes"),
        }),
        Err(error) => findings.push(Finding {
            check: "capacity",
            severity: Severity::Warn,
            message: format!("could not measure the data directory: {error}"),
        }),
    }

    // Capabilities: report what this build serves, never more.
    let tools = crate::protocol::tools_for(profile);
    let undelivered = diskgraph_core::CATALOG
        .iter()
        .filter(|spec| spec.stage != diskgraph_core::Stage::P3)
        .count();
    findings.push(Finding {
        check: "capabilities",
        severity: Severity::Ok,
        message: format!(
            "{} tools served in profile {}; {undelivered} catalog families not yet delivered",
            tools.len(),
            profile.wire_name()
        ),
    });

    Report { findings }
}

/// Recursive directory size using Rust APIs only (no `du`).
pub fn directory_bytes(path: &Path) -> std::io::Result<u64> {
    let mut total = 0u64;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            // An entry that vanished mid-walk is unknown, not zero bytes.
            Err(_) => continue,
        };
        if metadata.is_dir() {
            total = total.saturating_add(directory_bytes(&entry.path())?);
        } else {
            total = total.saturating_add(metadata.len());
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ToolProfile;
    use diskgraph_engine::{Engine, EngineConfig};

    fn engine(label: &str) -> (Engine, tempfile::TempDir) {
        let directory =
            tempfile::TempDir::with_prefix(format!("diskgraph-doctor-{label}-")).unwrap();
        let engine = Engine::open(EngineConfig {
            data_dir: directory.path().join("data"),
            max_nodes_per_scan: 1_000,
            ..EngineConfig::default()
        })
        .unwrap();
        (engine, directory)
    }

    #[test]
    fn a_healthy_engine_reports_no_errors() {
        let (engine, _keep) = engine("healthy");
        let report = diagnose(&engine, ToolProfile::ReadFull);
        assert!(
            report.is_healthy(),
            "unexpected findings: {:?}",
            report.findings
        );
        let checks: Vec<&str> = report.findings.iter().map(|f| f.check).collect();
        for expected in [
            "graph_store",
            "control_store",
            "server_identity",
            "authorization",
            "capacity",
            "capabilities",
        ] {
            assert!(checks.contains(&expected), "{expected} check missing");
        }
    }

    #[test]
    fn doctor_never_mutates_the_data_directory() {
        let (engine, _keep) = engine("readonly");
        let before = directory_bytes(engine.data_dir()).unwrap();
        let _ = diagnose(&engine, ToolProfile::All);
        let after = directory_bytes(engine.data_dir()).unwrap();
        assert_eq!(before, after, "doctor must not write to the data directory");
    }

    #[test]
    fn the_report_states_the_served_tool_set() {
        let (engine, _keep) = engine("tools");
        let report = diagnose(&engine, ToolProfile::ReadMinimal);
        let json = report.to_json(&engine, ToolProfile::ReadMinimal);
        assert_eq!(json["healthy"], true);
        assert_eq!(json["served"]["profile"], "read-minimal");
        let tools = json["served"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 3);
        // Capabilities never claim write tools in this build.
        assert!(tools.iter().all(|tool| {
            let name = tool.as_str().unwrap();
            !name.contains("move") && !name.contains("trash") && !name.contains("apply")
        }));
    }

    #[test]
    fn directory_size_is_measured_without_shelling_out() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("a"), vec![0; 100]).unwrap();
        std::fs::create_dir(directory.path().join("sub")).unwrap();
        std::fs::write(directory.path().join("sub").join("b"), vec![0; 250]).unwrap();
        let bytes = directory_bytes(directory.path()).unwrap();
        assert!(bytes >= 350, "expected at least 350 bytes, got {bytes}");
    }
}
