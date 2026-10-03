//! Lockfile contract shared by native package delivery and module resolution.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Version, tarball URL, dependency metadata, tarball SHA-256.
pub type LockEntry = (String, String, serde_json::Value, String);

/// Legacy pm stored name@version descriptors; native pins store just version.
pub fn version_pin<'a>(name: &str, pin: &'a str) -> &'a str {
    pin.strip_prefix(&format!("{name}@")).unwrap_or(pin)
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Lock {
    #[serde(rename = "lockfileVersion")]
    pub version: u32,
    pub packages: BTreeMap<String, LockEntry>,
}
impl Default for Lock {
    fn default() -> Self {
        Self {
            version: 1,
            packages: BTreeMap::new(),
        }
    }
}
impl Lock {
    pub(crate) fn pins(self) -> BTreeMap<String, String> {
        self.packages
            .into_iter()
            .map(|(name, (pin, _, _, _))| {
                let version = version_pin(&name, &pin).to_owned();
                (name, version)
            })
            .collect()
    }
}
