#[derive(Debug, Deserialize)]
pub struct PublishPackageRequest {
    pub name: String,
    pub version: String,
    pub repo: String,
    pub git_ref: Option<String>,
    pub description: Option<String>,
    pub manifest: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct PackageRelease {
    pub package_name: String,
    pub version: String,
    pub owner: String,
    pub repo: String,
    pub git_ref: String,
    pub description: Option<String>,
    pub manifest: Option<String>,
    pub api_snapshot: Option<String>,
    pub api_change_kind: Option<String>,
    pub required_bump: Option<String>,
    pub capability_metadata: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PackageSummary {
    pub name: String,
    pub versions: Vec<String>,
    pub latest: Option<String>,
}

#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ApiChangeKind {
    Initial,
    Patch,
    Minor,
    Major,
}

#[derive(Debug, Serialize, Clone)]
pub struct PublishPreflight {
    pub package_name: String,
    pub requested_version: String,
    pub previous_version: Option<String>,
    pub detected_change: ApiChangeKind,
    pub required_bump: ApiChangeKind,
    pub minimum_allowed_version: String,
    pub allowed: bool,
    pub reasons: Vec<String>,
    pub issues: Vec<ApiValidationIssue>,
    pub capabilities: CapabilityReport,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Default)]
pub struct ApiSnapshot {
    pub exports: BTreeMap<String, ExportSignature>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct ExportSignature {
    pub kind: String,
    pub signature: String,
    pub source: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub examples: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ApiValidationIssue {
    pub code: String,
    pub severity: String,
    pub symbol: String,
    pub message: String,
    pub old_source: Option<String>,
    pub new_source: Option<String>,
    pub old_signature: Option<String>,
    pub new_signature: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct CapabilityReport {
    pub detected: Vec<String>,
    pub declared: Vec<String>,
    pub missing: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct PackageDocsResponse {
    pub package_name: String,
    pub version: String,
    pub symbols: Vec<DocSymbol>,
}

#[derive(Debug, Serialize, Clone)]
pub struct DocSymbol {
    pub symbol: String,
    pub kind: String,
    pub signature: String,
    pub source: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub examples: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ReleaseTreeResponse {
    pub package_name: String,
    pub version: String,
    pub git_ref: String,
    pub entries: Vec<TreeEntry>,
}

#[derive(Debug, Serialize, Clone)]
pub struct TreeEntry {
    pub mode: String,
    pub kind: String,
    pub object: String,
    pub size: Option<u64>,
    pub path: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct BlobResponse {
    pub package_name: String,
    pub version: String,
    pub path: String,
    pub git_ref: String,
    pub content: String,
}

impl ApiChangeKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Initial => "initial",
            Self::Patch => "patch",
            Self::Minor => "minor",
            Self::Major => "major",
        }
    }
}
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::collections::BTreeMap;
