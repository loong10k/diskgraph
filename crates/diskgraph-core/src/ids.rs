//! Server-issued identity contracts (P0 task 1.3, spec SC-02).
//!
//! IDs name things; they never grant access. A reference that skips any of
//! server/scope/revision/resource identity is forged input, not a handle.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An identifier issued by one DiskGraph server: 1-64 chars of `[0-9A-Za-z_-]`,
/// starting alphanumeric. Anything else must be rejected before storage.
fn validate_id(value: &str) -> Result<(), InvalidId> {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return Err(InvalidId(value.to_owned()));
    }
    let first = bytes[0];
    if !first.is_ascii_alphanumeric() {
        return Err(InvalidId(value.to_owned()));
    }
    if !bytes[1..]
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || *b == b'-' || *b == b'_')
    {
        return Err(InvalidId(value.to_owned()));
    }
    Ok(())
}

/// Returned when a candidate identifier fails the server-issued ID contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvalidId(pub String);

impl core::fmt::Display for InvalidId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "invalid server-issued identifier: {:?}", self.0)
    }
}

impl std::error::Error for InvalidId {}

macro_rules! define_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
        pub struct $name(String);

        impl $name {
            /// Validates and wraps a candidate identifier.
            pub fn new(value: impl Into<String>) -> Result<Self, InvalidId> {
                let value = value.into();
                validate_id(&value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl TryFrom<String> for $name {
            type Error = InvalidId;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = InvalidId;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }
    };
}

define_id! {
    /// Identity of one DiskGraph service instance; persisted server-side, never client-asserted.
    ServerId
}
define_id! {
    /// An admin-registered root/volume/provider scope within one server.
    ScopeId
}
define_id! {
    /// A mapped human or machine principal; authorization keys on this, not on paths.
    PrincipalId
}
define_id! {
    /// A published graph revision; queries and plans bind to one and never mix.
    RevisionId
}

/// A fully qualified observation reference (design D3): server + scope + revision + node.
///
/// Node IDs are scoped to one revision and serialize as decimal strings so
/// cross-language consumers cannot truncate them (spec Q-07 / PF-01).
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ResourceRef {
    pub server_id: ServerId,
    pub scope_id: ScopeId,
    pub revision_id: RevisionId,
    pub node_id: u64,
}

impl ResourceRef {
    /// True only when every identity component matches; a same-named path on a
    /// different server or scope is a different resource.
    pub fn same_resource(&self, other: &ResourceRef) -> bool {
        self == other
    }
}

impl Serialize for ResourceRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("ResourceRef", 4)?;
        state.serialize_field("server_id", &self.server_id)?;
        state.serialize_field("scope_id", &self.scope_id)?;
        state.serialize_field("revision_id", &self.revision_id)?;
        state.serialize_field("node_id", &self.node_id.to_string())?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for ResourceRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            server_id: ServerId,
            scope_id: ScopeId,
            revision_id: RevisionId,
            node_id: String,
        }
        let raw = Raw::deserialize(deserializer)?;
        let node_id = raw.node_id.parse::<u64>().map_err(|_| {
            serde::de::Error::custom(format!("node_id is not a decimal string: {}", raw.node_id))
        })?;
        Ok(ResourceRef {
            server_id: raw.server_id,
            scope_id: raw.scope_id,
            revision_id: raw.revision_id,
            node_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(value: &str) -> ServerId {
        ServerId::new(value).unwrap()
    }

    fn scope(value: &str) -> ScopeId {
        ScopeId::new(value).unwrap()
    }

    fn revision(value: &str) -> RevisionId {
        RevisionId::new(value).unwrap()
    }

    #[test]
    fn identifiers_reject_empty_overlong_and_special_characters() {
        assert!(ServerId::new("").is_err());
        assert!(ServerId::new("-leading").is_err());
        assert!(ServerId::new("has space").is_err());
        assert!(ServerId::new("has/slash").is_err());
        assert!(ServerId::new("a".repeat(65)).is_err());
        assert!(ServerId::new("a".repeat(64)).is_ok());
        assert!(ServerId::new("srv-1_alpha").is_ok());
        assert!(ScopeId::new("0scope").is_ok());
    }

    #[test]
    fn same_named_paths_across_servers_are_distinct_resources() {
        let on_a = ResourceRef {
            server_id: server("server-a"),
            scope_id: scope("project"),
            revision_id: revision("rev-1"),
            node_id: 7,
        };
        let on_b = ResourceRef {
            server_id: server("server-b"),
            scope_id: scope("project"),
            revision_id: revision("rev-1"),
            node_id: 7,
        };
        assert!(!on_a.same_resource(&on_b));
        assert!(on_a.same_resource(&on_a.clone()));
    }

    #[test]
    fn revision_or_scope_change_makes_a_reference_incomparable() {
        let base = ResourceRef {
            server_id: server("s"),
            scope_id: scope("project"),
            revision_id: revision("rev-1"),
            node_id: 7,
        };
        let other_revision = ResourceRef {
            revision_id: revision("rev-2"),
            ..base.clone()
        };
        let other_scope = ResourceRef {
            scope_id: scope("other"),
            ..base.clone()
        };
        assert!(!base.same_resource(&other_revision));
        assert!(!base.same_resource(&other_scope));
    }

    #[test]
    fn node_id_round_trips_as_decimal_string_and_rejects_other_shapes() {
        let reference = ResourceRef {
            server_id: server("s"),
            scope_id: scope("project"),
            revision_id: revision("rev-1"),
            node_id: u64::MAX,
        };
        let json = serde_json::to_string(&reference).unwrap();
        assert!(json.contains("\"node_id\":\"18446744073709551615\""));
        let decoded: ResourceRef = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, reference);
        let forged = json.replace("18446744073709551615", "not-a-number");
        assert!(serde_json::from_str::<ResourceRef>(&forged).is_err());
    }
}
