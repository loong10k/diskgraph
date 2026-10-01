//! Remote authentication for the HTTP transport (P4 task 5.3, specs SC-05 /
//! MCP-06).
//!
//! The server is an OAuth resource server: it verifies a bearer token against
//! an issuer, an audience, an expiry, and a signature, then maps the subject to
//! a principal. It never trusts a proxy-supplied identity header, and a token
//! that fails any check yields no principal at all.

use hmac::Mac as _;
use std::collections::HashMap;

use diskgraph_core::{Authorizer as _, Permission, PolicyAuthorizer, PrincipalId};
use serde_json::{Value, json};

/// Claims a resource server needs from an access token.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenClaims {
    /// `iss`: who issued the token. A token from another issuer is refused.
    pub issuer: String,
    /// `aud`: who the token is for. A token minted for another service is
    /// refused, which is what stops a token for service A being replayed here.
    pub audience: String,
    /// `sub`: the human or machine the token speaks for.
    pub subject: String,
    /// `exp`: expiry in seconds since the epoch.
    pub expires_at_unix_seconds: u64,
    /// Optional scope string, mapped onto DiskGraph permissions.
    pub scope: Option<String>,
}

impl TokenClaims {
    /// Reads the claims from a decoded JWT payload. Returns `None` when a
    /// required claim is missing or of the wrong type: an incomplete token is
    /// not a partially trusted token.
    pub fn from_payload(payload: &Value) -> Option<Self> {
        Some(Self {
            issuer: payload.get("iss")?.as_str()?.to_owned(),
            audience: payload.get("aud")?.as_str()?.to_owned(),
            subject: payload.get("sub")?.as_str()?.to_owned(),
            expires_at_unix_seconds: payload.get("exp").and_then(Value::as_u64)?,
            scope: payload
                .get("scope")
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
    }
}

/// Why a token was refused. Each variant is reported without echoing the token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthFailure {
    Missing,
    Malformed,
    BadSignature,
    UnknownIssuer,
    WrongAudience,
    Expired,
    NoLeeway,
}

impl AuthFailure {
    /// Stable wire name; never carries token material.
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Missing => "missing_token",
            Self::Malformed => "malformed_token",
            Self::BadSignature => "invalid_signature",
            Self::UnknownIssuer => "unknown_issuer",
            Self::WrongAudience => "wrong_audience",
            Self::Expired => "expired_token",
            Self::NoLeeway => "clock_skew_exceeded",
        }
    }

    /// The HTTP status this failure maps to. A bad signature, issuer, audience,
    /// or expiry are all `401`; a valid token with no grant is `403` (handled
    /// separately by authorization).
    pub fn http_status(self) -> u16 {
        401
    }
}

/// One issuer the server accepts, with the key it verifies that issuer's
/// signatures and the audience it must name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssuerConfig {
    pub issuer: String,
    pub audience: String,
    /// The shared secret (or HMAC key) for this issuer's signatures.
    pub verification_key: Vec<u8>,
}

/// The authentication configuration a deployment publishes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthConfig {
    /// Accepted issuers; a token from any other issuer is refused.
    pub issuers: Vec<IssuerConfig>,
    /// Tolerance for clock skew between the server and the issuer.
    pub clock_skew_seconds: u64,
    /// Whether a token without any recognised scope maps to any permission.
    /// Off by default: a token with no scope grants nothing.
    pub unscoped_token_grants_metadata: bool,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            issuers: Vec::new(),
            clock_skew_seconds: 30,
            unscoped_token_grants_metadata: false,
        }
    }
}

impl AuthConfig {
    /// Builds a single-issuer configuration.
    pub fn single(issuer: &str, audience: &str, key: &[u8]) -> Self {
        Self {
            issuers: vec![IssuerConfig {
                issuer: issuer.to_owned(),
                audience: audience.to_owned(),
                verification_key: key.to_vec(),
            }],
            ..Self::default()
        }
    }

    /// A deployment with no accepted issuer authenticates nobody: every remote
    /// request is refused until an issuer is configured.
    pub fn rejects_everything(&self) -> bool {
        self.issuers.is_empty()
    }
}

/// The identity established for one authenticated request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthenticatedPrincipal {
    pub principal: PrincipalId,
    pub issuer: String,
    /// Permissions the token's scope maps to. An empty set authorizes nothing.
    pub permissions: Vec<Permission>,
    /// 长连接到期边界，不使用认证时钟偏差延长连接生命周期。
    pub expires_at_unix_seconds: u64,
}

/// Verifies bearer tokens for the HTTP transport. Verification is
/// deterministic and cheap enough to run per request, so nothing is cached.
#[derive(Clone)]
pub struct Authenticator {
    config: AuthConfig,
    /// Injected clock, so expiry and skew are testable without sleeping. An
    /// `Arc` keeps the authenticator cheap to share across connections.
    now_unix_seconds: std::sync::Arc<dyn Fn() -> u64 + Send + Sync>,
}

impl Authenticator {
    pub fn new(config: AuthConfig) -> Self {
        Self {
            config,
            now_unix_seconds: std::sync::Arc::new(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_secs())
                    .unwrap_or(0)
            }),
        }
    }

    /// Overrides the clock so expiry and skew are testable.
    pub fn with_clock(mut self, clock: impl Fn() -> u64 + Send + Sync + 'static) -> Self {
        self.now_unix_seconds = std::sync::Arc::new(clock);
        self
    }

    pub fn config(&self) -> &AuthConfig {
        &self.config
    }

    /// Extracts the bearer token from an Authorization header value.
    pub fn bearer_from_header(value: Option<&str>) -> Option<&str> {
        let value = value?.trim();
        let (scheme, token) = value.split_once(' ')?;
        if !scheme.eq_ignore_ascii_case("bearer") {
            return None;
        }
        let token = token.trim();
        (!token.is_empty()).then_some(token)
    }

    /// Verifies a token and maps it to a principal. Every failure returns
    /// `None` together with a reason and never a partial identity.
    pub fn authenticate(&self, token: Option<&str>) -> Result<AuthenticatedPrincipal, AuthFailure> {
        let token = token.ok_or(AuthFailure::Missing)?;
        let (header_segment, payload_segment, signature_segment) =
            split_jwt(token).ok_or(AuthFailure::Malformed)?;
        let header = decode_segment(header_segment).ok_or(AuthFailure::Malformed)?;
        let payload = decode_segment(payload_segment).ok_or(AuthFailure::Malformed)?;
        let signature = decode_bytes(signature_segment).ok_or(AuthFailure::Malformed)?;
        // Only HMAC tokens are accepted in this build; `alg: none` and any
        // asymmetric placeholder are refused before any claim is read.
        if header.get("alg").and_then(Value::as_str) != Some("HS256") {
            return Err(AuthFailure::Malformed);
        }
        let claims = TokenClaims::from_payload(&payload).ok_or(AuthFailure::Malformed)?;

        let issuer_config = self
            .config
            .issuers
            .iter()
            .find(|candidate| candidate.issuer == claims.issuer)
            .ok_or(AuthFailure::UnknownIssuer)?;
        // Audience is checked against the issuer's configured resource, not
        // against whatever the token claims for itself.
        if claims.audience != issuer_config.audience {
            return Err(AuthFailure::WrongAudience);
        }
        let now = (self.now_unix_seconds)();
        if claims.expires_at_unix_seconds <= now.saturating_sub(self.config.clock_skew_seconds) {
            return Err(AuthFailure::Expired);
        }
        let signing_input = format!("{header_segment}.{payload_segment}");
        let expected = hmac_sha256(&issuer_config.verification_key, signing_input.as_bytes());
        if !constant_time_eq(&expected, &signature) {
            return Err(AuthFailure::BadSignature);
        }

        let principal = PrincipalId::new(subject_to_principal(&claims.issuer, &claims.subject))
            .map_err(|_| AuthFailure::Malformed)?;
        Ok(AuthenticatedPrincipal {
            principal,
            expires_at_unix_seconds: claims.expires_at_unix_seconds,
            issuer: claims.issuer,
            permissions: permissions_from_scope(claims.scope.as_deref(), &self.config),
        })
    }

    /// Decides one capability for an authenticated principal against the live
    /// policy. Authentication alone grants nothing.
    pub fn authorize(
        &self,
        identity: &AuthenticatedPrincipal,
        permission: &Permission,
        scope: &diskgraph_core::ScopeId,
        authorizer: &PolicyAuthorizer,
    ) -> bool {
        // A permission the token's scope does not carry is refused even when
        // the policy would allow it: the token narrows the principal.
        if !identity.permissions.contains(permission) {
            return false;
        }
        matches!(
            authorizer.decide(&identity.principal, permission, scope),
            diskgraph_core::Decision::Allowed
        )
    }
}

/// Maps a token scope string onto DiskGraph permissions. Unknown scope values
/// are ignored rather than treated as a wildcard.
pub fn permissions_from_scope(scope: Option<&str>, config: &AuthConfig) -> Vec<Permission> {
    let Some(scope) = scope else {
        return if config.unscoped_token_grants_metadata {
            vec![Permission::MetadataRead]
        } else {
            Vec::new()
        };
    };
    let mut permissions = Vec::new();
    for entry in scope.split_whitespace() {
        match entry {
            "metadata:read" => permissions.push(Permission::MetadataRead),
            "content:read" => permissions.push(Permission::ContentRead),
            "index:write" => permissions.push(Permission::IndexWrite),
            "scope:admin" => permissions.push(Permission::ScopeAdmin),
            "operations:view" => permissions.push(Permission::OperationView),
            "files:move" => {
                permissions.push(Permission::FileAction(diskgraph_core::FileActionKind::Move))
            }
            "files:copy" => {
                permissions.push(Permission::FileAction(diskgraph_core::FileActionKind::Copy))
            }
            "files:trash" => permissions.push(Permission::FileAction(
                diskgraph_core::FileActionKind::Trash,
            )),
            "files:restore" => permissions.push(Permission::FileAction(
                diskgraph_core::FileActionKind::Restore,
            )),
            "files:purge" => permissions.push(Permission::FileAction(
                diskgraph_core::FileActionKind::Purge,
            )),
            // An unrecognised scope value grants nothing; it is not an error,
            // because a broader authorization server may issue scopes this
            // build does not use yet.
            _ => {}
        }
    }
    permissions
}

/// Derives a stable principal id from a token subject.
fn subject_to_principal(issuer: &str, subject: &str) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update((issuer.len() as u64).to_be_bytes());
    hasher.update(issuer.as_bytes());
    hasher.update(subject.as_bytes());
    format!("subject-{:x}", hasher.finalize())[..64].to_owned()
}

/// Splits a compact JWS into its three segments.
fn split_jwt(token: &str) -> Option<(&str, &str, &str)> {
    let mut parts = token.split('.');
    let header = parts.next()?;
    let payload = parts.next()?;
    let signature = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    (!header.is_empty() && !payload.is_empty() && !signature.is_empty())
        .then_some((header, payload, signature))
}

/// Decodes one base64url segment into JSON.
fn decode_segment(segment: &str) -> Option<Value> {
    serde_json::from_slice(&decode_bytes(segment)?).ok()
}

/// Decodes one base64url segment into raw bytes (a signature is not JSON).
fn decode_bytes(segment: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    URL_SAFE_NO_PAD.decode(segment.as_bytes()).ok()
}

/// Encodes one value as a base64url segment (used by tests and by a trusted
/// local token minter).
pub fn encode_segment(value: &Value) -> String {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(value).unwrap_or_default())
}

/// HMAC-SHA-256 over `message` with `key`. Delegated to the RustCrypto
/// `hmac`/`sha2` crates: cryptographic primitives are audited upstream, not
/// re-typed here. The signature is unchanged, so every caller and every
/// published test vector exercises the same contract.
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> Vec<u8> {
    type HmacSha256 = hmac::Hmac<sha2::Sha256>;
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(message);
    mac.finalize().into_bytes().to_vec()
}

/// SHA-256 of one message (RustCrypto `sha2`, same reason as above).
pub fn sha256(message: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(message);
    hasher.finalize().into()
}

/// Compares two byte strings without an early exit on the first difference.
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for index in 0..left.len() {
        difference |= left[index] ^ right[index];
    }
    difference == 0
}

/// A token minter for tests and for a trusted local deployment. It is not a
/// credential authority: production tokens come from the configured issuer.
pub struct TokenMinter {
    key: Vec<u8>,
}

impl TokenMinter {
    pub fn new(key: &[u8]) -> Self {
        Self { key: key.to_vec() }
    }

    /// Signs a token with the given claims.
    pub fn mint(&self, claims: &TokenClaims) -> String {
        let header = encode_segment(&json!({"alg": "HS256", "typ": "JWT"}));
        let payload = encode_segment(&json!({
            "iss": claims.issuer,
            "aud": claims.audience,
            "sub": claims.subject,
            "exp": claims.expires_at_unix_seconds,
            "scope": claims.scope,
        }));
        let signature = hmac_sha256(&self.key, format!("{header}.{payload}").as_bytes());
        use base64::Engine as _;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        format!("{header}.{payload}.{}", URL_SAFE_NO_PAD.encode(signature))
    }
}

/// The 401 body for a refused request: the reason and nothing else. No token
/// material, no issuer configuration, and no index data appear here.
pub fn unauthorized_body(failure: AuthFailure) -> Value {
    json!({
        "error": "unauthorized",
        "reason": failure.wire_name(),
    })
}

/// Extracts a bearer token from request headers, ignoring any proxy identity
/// header. Only `Authorization: Bearer` establishes identity.
pub fn token_from_headers(headers: &HashMap<String, String>) -> Option<String> {
    Authenticator::bearer_from_header(headers.get("authorization").map(String::as_str))
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use diskgraph_core::ScopeId;

    const KEY: &[u8] = b"test-verification-key";
    const ISSUER: &str = "https://auth.example.test";
    const AUDIENCE: &str = "diskgraph";
    const NOW: u64 = 1_700_000_000;

    fn claims() -> TokenClaims {
        TokenClaims {
            issuer: ISSUER.into(),
            audience: AUDIENCE.into(),
            subject: "agent@example.test".into(),
            expires_at_unix_seconds: NOW + 3_600,
            scope: Some("metadata:read".into()),
        }
    }

    fn authenticator() -> Authenticator {
        Authenticator::new(AuthConfig::single(ISSUER, AUDIENCE, KEY)).with_clock(move || NOW)
    }

    fn token() -> String {
        TokenMinter::new(KEY).mint(&claims())
    }

    fn identity(authenticator: &Authenticator, token: &str) -> AuthenticatedPrincipal {
        authenticator
            .authenticate(Some(token))
            .expect("token accepted")
    }

    #[test]
    fn a_well_formed_signed_token_authenticates() {
        let authenticator = authenticator();
        let principal = identity(&authenticator, &token());
        assert!(principal.principal.as_str().starts_with("subject-"));
        assert_eq!(principal.issuer, ISSUER);
        assert_eq!(principal.permissions, vec![Permission::MetadataRead]);
    }

    #[test]
    fn a_missing_token_is_refused() {
        let authenticator = authenticator();
        assert_eq!(
            authenticator.authenticate(None).unwrap_err(),
            AuthFailure::Missing
        );
        assert_eq!(
            Authenticator::bearer_from_header(None),
            None,
            "no header is no token"
        );
        assert_eq!(
            Authenticator::bearer_from_header(Some("Basic abc")),
            None,
            "a non-bearer scheme is not a bearer token"
        );
    }

    #[test]
    fn a_forged_signature_is_refused() {
        let authenticator = authenticator();
        // Same claims, signed with the wrong key.
        let forged = TokenMinter::new(b"attacker-key").mint(&claims());
        assert_eq!(
            authenticator.authenticate(Some(&forged)).unwrap_err(),
            AuthFailure::BadSignature
        );
        // A tampered payload under a real signature also fails.
        let valid = token();
        let mut parts = valid.split('.');
        let header = parts.next().unwrap();
        let _payload = parts.next().unwrap();
        let signature = parts.next().unwrap();
        let escalated = encode_segment(&json!({
            "iss": ISSUER, "aud": AUDIENCE, "sub": "agent@example.test",
            "exp": NOW + 3_600, "scope": "scope:admin index:write metadata:read"
        }));
        let tampered = format!("{header}.{escalated}.{signature}");
        assert_eq!(
            authenticator.authenticate(Some(&tampered)).unwrap_err(),
            AuthFailure::BadSignature
        );
    }

    #[test]
    fn a_token_from_an_unknown_issuer_is_refused() {
        let authenticator = authenticator();
        let foreign = TokenClaims {
            issuer: "https://evil.example.test".into(),
            ..claims()
        };
        let token = TokenMinter::new(KEY).mint(&foreign);
        assert_eq!(
            authenticator.authenticate(Some(&token)).unwrap_err(),
            AuthFailure::UnknownIssuer
        );
    }

    #[test]
    fn a_token_minted_for_another_audience_is_refused() {
        let authenticator = authenticator();
        let other = TokenClaims {
            audience: "some-other-service".into(),
            ..claims()
        };
        let token = TokenMinter::new(KEY).mint(&other);
        assert_eq!(
            authenticator.authenticate(Some(&token)).unwrap_err(),
            AuthFailure::WrongAudience
        );
    }

    #[test]
    fn an_expired_token_is_refused() {
        let authenticator = authenticator();
        let expired = TokenClaims {
            expires_at_unix_seconds: NOW - 1_000,
            ..claims()
        };
        let token = TokenMinter::new(KEY).mint(&expired);
        assert_eq!(
            authenticator.authenticate(Some(&token)).unwrap_err(),
            AuthFailure::Expired
        );
    }

    #[test]
    fn a_token_inside_the_skew_window_is_still_accepted() {
        // 10 seconds past expiry, with 30 seconds of configured tolerance.
        let authenticator = authenticator();
        let just_expired = TokenClaims {
            expires_at_unix_seconds: NOW - 10,
            ..claims()
        };
        let token = TokenMinter::new(KEY).mint(&just_expired);
        assert!(authenticator.authenticate(Some(&token)).is_ok());
    }

    #[test]
    fn malformed_tokens_and_unsigned_ones_are_refused() {
        let authenticator = authenticator();
        for token in ["not-a-jwt", "a.b", "a.b.c.d", "", "..."] {
            assert_eq!(
                authenticator.authenticate(Some(token)).unwrap_err(),
                AuthFailure::Malformed,
                "{token:?} must be refused as malformed"
            );
        }
        // `alg: none` is refused before any claim is read.
        let header = encode_segment(&json!({"alg": "none"}));
        let payload = encode_segment(&json!({
            "iss": ISSUER, "aud": AUDIENCE, "sub": "admin",
            "exp": NOW + 3_600, "scope": "scope:admin"
        }));
        let none_token = format!("{header}.{payload}.");
        assert_eq!(
            authenticator.authenticate(Some(&none_token)).unwrap_err(),
            AuthFailure::Malformed
        );
    }

    #[test]
    fn a_token_missing_a_required_claim_is_refused() {
        let authenticator = authenticator();
        // No `aud`.
        let header = encode_segment(&json!({"alg": "HS256"}));
        let payload = encode_segment(&json!({
            "iss": ISSUER, "sub": "agent", "exp": NOW + 3_600
        }));
        let signing_input = format!("{header}.{payload}");
        let signature = hmac_sha256(KEY, signing_input.as_bytes());
        use base64::Engine as _;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let token = format!("{signing_input}.{}", URL_SAFE_NO_PAD.encode(signature));
        assert_eq!(
            authenticator.authenticate(Some(&token)).unwrap_err(),
            AuthFailure::Malformed
        );
    }

    #[test]
    fn a_deployment_with_no_issuer_authenticates_nobody() {
        let authenticator = Authenticator::new(AuthConfig::default()).with_clock(move || NOW);
        assert!(authenticator.config().rejects_everything());
        let token = token();
        assert_eq!(
            authenticator.authenticate(Some(&token)).unwrap_err(),
            AuthFailure::UnknownIssuer
        );
    }

    #[test]
    fn authentication_alone_grants_no_permission() {
        let authenticator = authenticator();
        let principal = identity(&authenticator, &token());
        let scope = ScopeId::new("project").unwrap();
        // Authentication alone grants nothing: a bare policy with no grants
        // refuses every capability, even one the token's scope carries.
        let policy = PolicyAuthorizer::new(1);
        assert!(!authenticator.authorize(&principal, &Permission::MetadataRead, &scope, &policy));

        // With a policy grant for metadata read, the carried scope is
        // authorized...
        let mut policy = PolicyAuthorizer::new(1);
        policy.grant(
            principal.principal.clone(),
            Permission::MetadataRead,
            scope.clone(),
        );
        assert!(authenticator.authorize(&principal, &Permission::MetadataRead, &scope, &policy));

        // ...but the same principal with the same policy still cannot act as
        // an index writer: the token's scope does not carry index:write, so
        // the policy grant it does have must not leak into a wider capability.
        let mut policy = PolicyAuthorizer::new(1);
        policy.grant(
            principal.principal.clone(),
            Permission::IndexWrite,
            scope.clone(),
        );
        assert!(
            !authenticator.authorize(&principal, &Permission::IndexWrite, &scope, &policy),
            "a metadata-only token must not act as an index writer"
        );
    }

    #[test]
    fn a_token_without_scope_grants_nothing_by_default() {
        let config = AuthConfig::single(ISSUER, AUDIENCE, KEY);
        assert!(permissions_from_scope(None, &config).is_empty());
        let permissive = AuthConfig {
            unscoped_token_grants_metadata: true,
            ..config.clone()
        };
        assert_eq!(
            permissions_from_scope(None, &permissive),
            vec![Permission::MetadataRead]
        );
    }

    #[test]
    fn unknown_scope_values_are_ignored_rather_than_widened() {
        let config = AuthConfig::single(ISSUER, AUDIENCE, KEY);
        let permissions = permissions_from_scope(Some("metadata:read admin:everything"), &config);
        assert_eq!(permissions, vec![Permission::MetadataRead]);
    }

    #[test]
    fn the_unauthorized_body_never_echoes_token_or_index_material() {
        let body = unauthorized_body(AuthFailure::BadSignature).to_string();
        assert!(!body.contains("eyJ"), "no JWT fragment appears");
        assert!(body.contains("invalid_signature"));
        assert!(!body.contains('/'), "no path appears");
    }

    #[test]
    fn proxy_identity_headers_are_never_trusted() {
        let mut headers = HashMap::new();
        headers.insert("x-forwarded-user".into(), "root".into());
        headers.insert("x-auth-request-user".into(), "root".into());
        assert_eq!(
            token_from_headers(&headers),
            None,
            "identity must come from a bearer token, never a proxy header"
        );
        headers.insert("authorization".into(), "Bearer real-token".into());
        assert_eq!(token_from_headers(&headers).as_deref(), Some("real-token"));
    }

    #[test]
    fn hmac_sha256_matches_the_published_test_vector() {
        // RFC 4231 test case 2.
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        let hex: String = mac.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(
            hex,
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn constant_time_comparison_rejects_different_lengths_and_values() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(constant_time_eq(b"", b""));
    }
}
