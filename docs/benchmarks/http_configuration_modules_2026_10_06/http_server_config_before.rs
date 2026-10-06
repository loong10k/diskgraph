/// Everything one HTTP listener needs: limits, security context, and the
/// legacy adapter gate (default off, P4 tasks 5.6/5.7).
pub struct ServerConfig {
    pub limits: HttpLimits,
    pub security: Security,
    pub legacy_sse: bool,
}

impl ServerConfig {
    /// The modern-transport server with defaults.
    pub fn modern(limits: HttpLimits, security: Security) -> Self {
        Self {
            limits,
            security,
            legacy_sse: false,
        }
    }

    /// The same server with the legacy adapter enabled.
    pub fn with_legacy(mut self) -> Self {
        self.legacy_sse = true;
        self
    }
}

