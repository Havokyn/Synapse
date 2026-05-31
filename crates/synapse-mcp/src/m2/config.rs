pub(super) const RECORDING_BACKEND_ENV: &str = "SYNAPSE_MCP_RECORDING_BACKEND";

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct M2ServiceConfig {
    pub recording_backend: Option<String>,
}

impl M2ServiceConfig {
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            recording_backend: std::env::var(RECORDING_BACKEND_ENV).ok(),
        }
    }
}
