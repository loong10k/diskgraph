//! Versioned v2 response envelope shared by CLI, MCP, and FFI (P0 task 1.5,
//! specs Q-01 / Q-07 / PF-01).
//!
//! Contract rules encoded here: `api_version` is always present; unknown or
//! cross-language-unsafe numbers are decimal strings; absence is `null`, never
//! `0`; and business errors carry a stable code plus the fixed exit code.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::errors::BusinessError;
use crate::ids::{ScopeId, ServerId};

/// The current public API version of the v2 envelope.
pub const API_VERSION: u32 = 2;

/// One structured result. Producers fill what they know; consumers must treat
/// missing optional fields as "not observed", never as zero.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub api_version: u32,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_id: Option<ServerId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<ScopeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<crate::ids::RevisionId>,
    /// RFC 3339 timestamp of evaluation; freshness must be judgeable from the response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluated_at: Option<String>,
    /// Per-observation-area state, e.g. `{"filesystem":"complete","process":"unsupported"}`.
    #[serde(default)]
    pub coverage: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<EnvelopeError>,
}

/// A structured business failure; protocol and transport errors never masquerade
/// as this shape and vice versa.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EnvelopeError {
    pub code: String,
    pub exit_code: u8,
    pub message: String,
    /// Redacted diagnostic reference for `internal_error`; no raw secrets or paths.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic_id: Option<String>,
}

impl Envelope {
    /// A successful result; `truncated` stays false unless the producer bounded output.
    pub fn ok(data: Value) -> Self {
        Self {
            api_version: API_VERSION,
            ok: true,
            server_id: None,
            scope_id: None,
            revision_id: None,
            evaluated_at: None,
            coverage: Value::Null,
            data: Some(data),
            warnings: Vec::new(),
            truncated: false,
            next_cursor: None,
            error: None,
        }
    }

    /// A business failure; `data` is absent so partial payloads cannot leak
    /// past a denied or failed request.
    pub fn failure(error: BusinessError, message: impl Into<String>) -> Self {
        Self {
            api_version: API_VERSION,
            ok: false,
            server_id: None,
            scope_id: None,
            revision_id: None,
            evaluated_at: None,
            coverage: Value::Null,
            data: None,
            warnings: Vec::new(),
            truncated: false,
            next_cursor: None,
            error: Some(EnvelopeError {
                code: error.code().to_owned(),
                exit_code: error.exit_code(),
                message: message.into(),
                diagnostic_id: None,
            }),
        }
    }

    /// Builder-style identity binding; responses must state which revision answered.
    pub fn with_ids(
        mut self,
        server_id: Option<ServerId>,
        scope_id: Option<ScopeId>,
        revision_id: Option<crate::ids::RevisionId>,
    ) -> Self {
        self.server_id = server_id;
        self.scope_id = scope_id;
        self.revision_id = revision_id;
        self
    }

    /// The envelope as a JSON value, for transports that embed it.
    pub fn into_json(self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_else(|_| {
            serde_json::to_value(Envelope::failure(
                crate::errors::BusinessError::InternalError,
                "envelope encoding failed",
            ))
            .unwrap_or(serde_json::Value::Null)
        })
    }

    /// Renders a byte count as the contract-mandated decimal string, or null.
    pub fn bytes_or_null(value: Option<u64>) -> Value {
        match value {
            Some(bytes) => json!(bytes.to_string()),
            None => Value::Null,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_envelope_carries_version_bindings_and_string_bytes() {
        let envelope = Envelope::ok(json!({
            "node_id": "42",
            "allocated_bytes": Envelope::bytes_or_null(Some(1_073_741_824)),
            "apparent_bytes": Envelope::bytes_or_null(None),
            "review_status": "unknown",
        }))
        .with_ids(
            Some(ServerId::new("server-a").unwrap()),
            Some(ScopeId::new("project").unwrap()),
            Some(crate::ids::RevisionId::new("rev-1").unwrap()),
        );
        let encoded = serde_json::to_value(&envelope).unwrap();
        assert_eq!(encoded["api_version"], 2);
        assert_eq!(encoded["ok"], true);
        assert_eq!(encoded["server_id"], "server-a");
        assert_eq!(encoded["data"]["allocated_bytes"], "1073741824");
        assert!(encoded["data"]["apparent_bytes"].is_null());
        let decoded: Envelope = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, envelope);
    }

    #[test]
    fn failure_envelope_exposes_code_exit_and_no_data() {
        let envelope = Envelope::failure(
            BusinessError::NotIndexed,
            "scope not indexed; run index first",
        );
        let encoded = serde_json::to_value(&envelope).unwrap();
        assert_eq!(encoded["ok"], false);
        assert_eq!(encoded["error"]["code"], "not_indexed");
        assert_eq!(encoded["error"]["exit_code"], 4);
        assert!(encoded.get("data").is_none());
    }

    #[test]
    fn truncation_is_reported_without_becoming_a_partial_failure() {
        let envelope = Envelope::ok(json!([]));
        let mut truncated = envelope.clone();
        truncated.truncated = true;
        truncated.next_cursor = Some("cursor-2".into());
        assert!(envelope.ok);
        assert!(truncated.ok && truncated.truncated && truncated.error.is_none());
    }
}
