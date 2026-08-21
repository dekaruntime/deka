use anyhow::Result;
use std::path::{Path, PathBuf};

pub trait Platform: Send + Sync {
    fn fs(&self) -> &dyn Fs;
    fn env(&self) -> &dyn Env;
    fn io(&self) -> &dyn Io;
    fn process(&self) -> &dyn Process;
    fn time(&self) -> &dyn Time;
    fn random(&self) -> &dyn Random;
    fn net(&self) -> &dyn Net;
    fn ports(&self) -> &dyn Ports;
}

pub trait Fs: Send + Sync {
    fn read(&self, path: &Path) -> Result<Vec<u8>>;
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<()>;
    fn create_dir_all(&self, path: &Path) -> Result<()>;
    fn exists(&self, path: &Path) -> bool;
    fn cwd(&self) -> Result<PathBuf>;
    fn canonicalize(&self, path: &Path) -> Result<PathBuf>;
    fn current_exe(&self) -> Result<PathBuf>;
}

pub trait Env: Send + Sync {
    fn get(&self, key: &str) -> Option<String>;
    fn vars(&self) -> Vec<(String, String)>;
    fn set(&self, key: &str, value: &str) -> Result<()>;
}

pub trait Io: Send + Sync {
    fn stdout(&self, message: &str) -> Result<()>;
    fn stderr(&self, message: &str) -> Result<()>;
}

pub trait Process: Send + Sync {
    fn args(&self) -> Vec<String>;
    fn pid(&self) -> u32;
    fn exit(&self, code: i32) -> !;
}

pub trait Time: Send + Sync {
    fn now_unix_ms(&self) -> i64;
    fn sleep_ms(&self, milliseconds: u64) -> Result<()>;
}

pub trait Random: Send + Sync {
    fn fill_bytes(&self, buffer: &mut [u8]) -> Result<()>;
}

pub trait Net: Send + Sync {
    fn fetch_bytes(&self, url: &str, method: &str, body: Option<&[u8]>) -> Result<Vec<u8>>;
}

pub trait Ports: Send + Sync {
    fn reserve(&self, preferred: Option<u16>) -> Result<u16>;
    fn release(&self, port: u16) -> Result<()>;
}
