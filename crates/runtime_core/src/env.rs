use std::collections::HashMap;

pub fn flag_or_env_truthy_with<F>(
    flags: &HashMap<String, bool>,
    long_flag: &str,
    short_flag: Option<&str>,
    env_var: &str,
    env_get: &F,
) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    if flags.contains_key(long_flag) {
        return true;
    }
    if let Some(short) = short_flag
        && flags.contains_key(short)
    {
        return true;
    }
    env_truthy_with(env_var, env_get)
}

pub fn env_truthy_with<F>(var: &str, env_get: &F) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    env_get(var).map(|value| is_truthy(&value)).unwrap_or(false)
}

pub fn is_truthy(value: &str) -> bool {
    matches!(value, "1" | "true" | "yes" | "on")
}

pub fn set_if_absent_with<Get, Set>(key: &str, value: &str, env_get: &Get, env_set: &mut Set)
where
    Get: Fn(&str) -> Option<String>,
    Set: FnMut(&str, &str),
{
    if env_get(key).is_some() {
        return;
    }
    env_set(key, value);
}

pub fn set_dev_flag_with<Get, Set>(enabled: bool, env_get: &Get, env_set: &mut Set)
where
    Get: Fn(&str) -> Option<String>,
    Set: FnMut(&str, &str),
{
    if enabled {
        set_if_absent_with("DEKA_DEV", "1", env_get, env_set);
    }
}

pub fn set_handler_path_with<Get, Set>(handler_path: &str, env_get: &Get, env_set: &mut Set)
where
    Get: Fn(&str) -> Option<String>,
    Set: FnMut(&str, &str),
{
    set_if_absent_with("HANDLER_PATH", handler_path, env_get, env_set);
}

pub fn set_runtime_args_with<Set, Bin>(extra_args: &[String], env_set: &mut Set, bin_name: &Bin)
where
    Set: FnMut(&str, &str),
    Bin: Fn() -> Option<String>,
{
    if !extra_args.is_empty()
        && let Ok(encoded) = serde_json::to_string(extra_args)
    {
        env_set("DEKA_ARGS", &encoded);
    }

    if let Some(bin) = bin_name() {
        env_set("DEKA_BIN", &bin);
    }
}
