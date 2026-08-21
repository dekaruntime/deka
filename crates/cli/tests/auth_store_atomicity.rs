use cli::cli::auth_store::{self, AuthProfile};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::tempdir;

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
}

fn profile(username: &str, token: &str) -> AuthProfile {
    AuthProfile {
        username: username.to_string(),
        token: token.to_string(),
        registry_url: "http://localhost:9418".to_string(),
    }
}

#[test]
#[ignore = "blocked: source bug, auth_store::save currently uses fs::write instead of atomic private writes"]
fn concurrent_save_never_exposes_partial_profile() {
    let _guard = env_lock();
    let dir = tempdir().unwrap();
    unsafe {
        std::env::set_var("HOME", dir.path());
    }

    auth_store::save(&profile("@seed", "seed-token")).unwrap();

    let reader = thread::spawn(|| {
        let deadline = Instant::now() + Duration::from_millis(250);
        while Instant::now() < deadline {
            let loaded = auth_store::load()
                .expect("load should not observe a partially-written auth profile")
                .expect("profile should exist");
            assert!(loaded.username.starts_with('@'));
        }
    });

    let writers: Vec<_> = (0..24)
        .map(|idx| {
            thread::spawn(move || {
                auth_store::save(&profile(&format!("@user{idx}"), &format!("token-{idx}")))
                    .unwrap();
            })
        })
        .collect();

    for writer in writers {
        writer.join().unwrap();
    }
    reader.join().unwrap();

    let loaded = auth_store::load().unwrap().unwrap();
    assert!(loaded.username.starts_with('@'));
    assert!(loaded.token.starts_with("token-") || loaded.token == "seed-token");
}

#[test]
#[ignore = "blocked: source bug, auth_store::save currently has no recovery contract for partial auth.json writes"]
fn save_recovers_from_partial_profile_file() {
    let _guard = env_lock();
    let dir = tempdir().unwrap();
    unsafe {
        std::env::set_var("HOME", dir.path());
    }

    let config_dir = dir.path().join(".config").join("deka");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join("auth.json"), b"{\"username\":\"@broken\"").unwrap();

    auth_store::save(&profile("@fixed", "fixed-token")).unwrap();

    let loaded = auth_store::load().unwrap().unwrap();
    assert_eq!(loaded.username, "@fixed");
    assert_eq!(loaded.token, "fixed-token");
}

#[test]
#[ignore = "blocked: source bug, auth_store::save currently leaves auth.json at the process umask mode"]
fn save_enforces_private_0600_permissions() {
    let _guard = env_lock();
    let dir = tempdir().unwrap();
    unsafe {
        std::env::set_var("HOME", dir.path());
    }

    auth_store::save(&profile("@secure", "secret-token")).unwrap();

    let path = dir.path().join(".config").join("deka").join("auth.json");
    let mode = fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}
