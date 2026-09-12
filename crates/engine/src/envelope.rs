pub use serve::request_envelope::StorefrontResponse as ResponseEnvelope;

pub type RequestEnvelope = LegacyRequestEnvelope;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct LegacyRequestEnvelope {
    pub url: String,
    pub method: String,
    pub headers: std::collections::HashMap<String, String>,
    pub body: Option<String>,
}
