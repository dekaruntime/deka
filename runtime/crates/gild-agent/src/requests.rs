use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub(crate) struct CreateAgentRequest {
    pub(crate) slug: String,
    pub(crate) persona_ref: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RemoveAgentRequest {
    pub(crate) slug: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RestartUnitRequest {
    pub(crate) unit: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SystemctlRequest {
    pub(crate) op: String,
    pub(crate) action: String,
    pub(crate) unit: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RotateHmacRequest {
    pub(crate) op: String,
    pub(crate) slug: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WriteUnitRequest {
    pub(crate) op: String,
    pub(crate) slug: String,
    pub(crate) kind: String,
    pub(crate) port: u16,
    #[serde(default)]
    pub(crate) extra_env: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DeleteUnitRequest {
    pub(crate) op: String,
    pub(crate) unit: String,
}
