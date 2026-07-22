pub(super) fn platform_dev_mode_enabled() -> bool {
    env_flag_enabled("DEKA_DEV_MODE")
        || env_flag_enabled("DEKA_DEV")
        || std::env::var("NODE_ENV").as_deref() == Ok("development")
}

fn env_flag_enabled(key: &str) -> bool {
    std::env::var(key)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

const PLATFORM_ENV_ALIASES: &[(&str, &str)] = &[
    ("NEO4J_URI", "DEKA_NEO4J_URI"),
    ("NEO4J_USER", "DEKA_NEO4J_USER"),
    ("NEO4J_PASSWORD", "DEKA_NEO4J_PASSWORD"),
    ("NEO4J_DB", "DEKA_NEO4J_DB"),
    ("REDIS_URL", "DEKA_REDIS_URL"),
];

fn platform_env_aliases_to_set<F>(env_get: F) -> Vec<(&'static str, String)>
where
    F: Fn(&str) -> Option<String>,
{
    PLATFORM_ENV_ALIASES
        .iter()
        .filter_map(|(source, target)| {
            if env_get(target).is_some() {
                return None;
            }
            env_get(source).map(|value| (*target, value))
        })
        .collect()
}

pub(super) fn install_platform_env_aliases() {
    let aliases = platform_env_aliases_to_set(|key| std::env::var(key).ok());
    if aliases.is_empty() {
        return;
    }

    // SAFETY: called during platform startup before request worker tasks are spawned.
    unsafe {
        for (target, value) in aliases {
            std::env::set_var(target, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{env_flag_enabled, platform_env_aliases_to_set};
    use std::collections::HashMap;

    #[test]
    fn env_flag_enabled_accepts_truthy_values() {
        unsafe { std::env::set_var("DEKA_RUNTIME_TEST_FLAG", "true") };
        assert!(env_flag_enabled("DEKA_RUNTIME_TEST_FLAG"));

        unsafe { std::env::set_var("DEKA_RUNTIME_TEST_FLAG", "0") };
        assert!(!env_flag_enabled("DEKA_RUNTIME_TEST_FLAG"));

        unsafe { std::env::remove_var("DEKA_RUNTIME_TEST_FLAG") };
        assert!(!env_flag_enabled("DEKA_RUNTIME_TEST_FLAG"));
    }

    #[test]
    fn platform_env_aliases_accept_container_contract_names() {
        let env = HashMap::from([
            ("NEO4J_URI", "bolt://neo4j:7687"),
            ("NEO4J_USER", "neo4j"),
            ("NEO4J_PASSWORD", "secret"),
            ("REDIS_URL", "redis://redis:6379"),
        ]);

        let aliases = platform_env_aliases_to_set(|key| env.get(key).map(|v| v.to_string()));

        assert_eq!(
            aliases,
            vec![
                ("DEKA_NEO4J_URI", "bolt://neo4j:7687".to_string()),
                ("DEKA_NEO4J_USER", "neo4j".to_string()),
                ("DEKA_NEO4J_PASSWORD", "secret".to_string()),
                ("DEKA_REDIS_URL", "redis://redis:6379".to_string()),
            ]
        );
    }

    #[test]
    fn platform_env_aliases_do_not_override_deka_specific_values() {
        let env = HashMap::from([
            ("NEO4J_URI", "bolt://wrong:7687"),
            ("DEKA_NEO4J_URI", "bolt://right:7687"),
            ("REDIS_URL", "redis://redis:6379"),
            ("DEKA_REDIS_URL", "redis://deka-redis:6379"),
        ]);

        let aliases = platform_env_aliases_to_set(|key| env.get(key).map(|v| v.to_string()));

        assert!(aliases.is_empty());
    }
}
