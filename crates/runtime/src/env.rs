pub(crate) fn init_env() {
    if std::env::var("DENO_PLATFORM").is_err() {
        let platform = match std::env::consts::OS {
            "macos" => "darwin",
            "windows" => "win32",
            other => other,
        };
        unsafe {
            std::env::set_var("DENO_PLATFORM", platform);
            std::env::set_var("DEKA_PLATFORM", platform);
        }
    }
    if std::env::var("DENO_ARCH").is_err() {
        let arch = match std::env::consts::ARCH {
            "aarch64" => "arm64",
            "x86_64" => "x64",
            other => other,
        };
        unsafe {
            std::env::set_var("DENO_ARCH", arch);
            std::env::set_var("DEKA_ARCH", arch);
        }
    }
    if std::env::var("PWD").is_err() {
        if let Ok(dir) = std::env::current_dir() {
            if let Some(dir_str) = dir.to_str() {
                unsafe {
                    std::env::set_var("PWD", dir_str);
                }
            }
        }
    }
    if std::env::var("INIT_CWD").is_err() {
        if let Ok(dir) = std::env::current_dir() {
            if let Some(dir_str) = dir.to_str() {
                unsafe {
                    std::env::set_var("INIT_CWD", dir_str);
                }
            }
        }
    }
}
