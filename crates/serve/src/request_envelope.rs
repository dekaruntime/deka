use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorefrontRequest {
    pub url: String,
    pub path: String,
    pub pathname: String,
    pub method: String,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorefrontResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: String,
    #[serde(default)]
    pub body_base64: Option<String>,
    #[serde(default)]
    pub upgrade: Option<serde_json::Value>,
}

impl StorefrontResponse {
    pub fn from_value(value: serde_json::Value) -> Result<Self, serde_json::Error> {
        serde_json::from_value(value)
    }
}
