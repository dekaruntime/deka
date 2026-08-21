use anyhow::{Result, anyhow};
use platform::{Env, Fs, Io, Net, Platform, Ports, Process, Random, Time};
use rand::RngCore;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct BrowserPlatform {
    fs: BrowserFs,
    env: BrowserEnv,
    io: BrowserIo,
    process: BrowserProcess,
    time: BrowserTime,
    random: BrowserRandom,
    net: BrowserNet,
    ports: BrowserPorts,
}

impl BrowserPlatform {
    pub fn new(allowlist: Vec<String>) -> Self {
        Self {
            fs: BrowserFs::default(),
            env: BrowserEnv::default(),
            io: BrowserIo::default(),
            process: BrowserProcess,
            time: BrowserTime,
            random: BrowserRandom,
            net: BrowserNet { allowlist },
            ports: BrowserPorts::default(),
        }
    }
}

impl Default for BrowserPlatform {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl Platform for BrowserPlatform {
    fn fs(&self) -> &dyn Fs {
        &self.fs
    }

    fn env(&self) -> &dyn Env {
        &self.env
    }

    fn io(&self) -> &dyn Io {
        &self.io
    }

    fn process(&self) -> &dyn Process {
        &self.process
    }

    fn time(&self) -> &dyn Time {
        &self.time
    }

    fn random(&self) -> &dyn Random {
        &self.random
    }

    fn net(&self) -> &dyn Net {
        &self.net
    }

    fn ports(&self) -> &dyn Ports {
        &self.ports
    }
}

#[derive(Default)]
pub struct BrowserFs {
    files: Mutex<HashMap<PathBuf, Vec<u8>>>,
}

impl Fs for BrowserFs {
    fn read(&self, path: &Path) -> Result<Vec<u8>> {
        let files = self
            .files
            .lock()
            .map_err(|_| anyhow!("failed to lock browser fs"))?;
        files
            .get(path)
            .cloned()
            .ok_or_else(|| anyhow!("file not found: {}", path.display()))
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        let mut files = self
            .files
            .lock()
            .map_err(|_| anyhow!("failed to lock browser fs"))?;
        files.insert(path.to_path_buf(), bytes.to_vec());
        Ok(())
    }

    fn create_dir_all(&self, _path: &Path) -> Result<()> {
        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        self.files
            .lock()
            .map(|files| files.contains_key(path))
            .unwrap_or(false)
    }

    fn cwd(&self) -> Result<PathBuf> {
        Ok(PathBuf::from("/"))
    }

    fn canonicalize(&self, path: &Path) -> Result<PathBuf> {
        Ok(path.to_path_buf())
    }

    fn current_exe(&self) -> Result<PathBuf> {
        Err(anyhow!("current_exe is unavailable in browser platform"))
    }
}

#[derive(Default)]
pub struct BrowserEnv {
    vars: Mutex<HashMap<String, String>>,
}

impl Env for BrowserEnv {
    fn get(&self, key: &str) -> Option<String> {
        self.vars
            .lock()
            .ok()
            .and_then(|vars| vars.get(key).cloned())
    }

    fn vars(&self) -> Vec<(String, String)> {
        self.vars
            .lock()
            .map(|vars| vars.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default()
    }

    fn set(&self, key: &str, value: &str) -> Result<()> {
        let mut vars = self
            .vars
            .lock()
            .map_err(|_| anyhow!("failed to lock browser env"))?;
        vars.insert(key.to_string(), value.to_string());
        Ok(())
    }
}

#[derive(Default)]
pub struct BrowserIo {
    stdout: Mutex<String>,
    stderr: Mutex<String>,
}

impl BrowserIo {
    pub fn stdout_buffer(&self) -> String {
        self.stdout.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn stderr_buffer(&self) -> String {
        self.stderr.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

impl Io for BrowserIo {
    fn stdout(&self, message: &str) -> Result<()> {
        let mut stdout = self
            .stdout
            .lock()
            .map_err(|_| anyhow!("failed to lock browser stdout"))?;
        stdout.push_str(message);
        Ok(())
    }

    fn stderr(&self, message: &str) -> Result<()> {
        let mut stderr = self
            .stderr
            .lock()
            .map_err(|_| anyhow!("failed to lock browser stderr"))?;
        stderr.push_str(message);
        Ok(())
    }
}

pub struct BrowserProcess;

impl Process for BrowserProcess {
    fn args(&self) -> Vec<String> {
        Vec::new()
    }

    fn pid(&self) -> u32 {
        1
    }

    fn exit(&self, code: i32) -> ! {
        panic!("browser process exit requested: {}", code)
    }
}

pub struct BrowserTime;

impl Time for BrowserTime {
    fn now_unix_ms(&self) -> i64 {
        let now = std::time::SystemTime::now();
        now.duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    fn sleep_ms(&self, milliseconds: u64) -> Result<()> {
        std::thread::sleep(std::time::Duration::from_millis(milliseconds));
        Ok(())
    }
}

pub struct BrowserRandom;

impl Random for BrowserRandom {
    fn fill_bytes(&self, buffer: &mut [u8]) -> Result<()> {
        let mut rng = rand::thread_rng();
        rng.fill_bytes(buffer);
        Ok(())
    }
}

pub struct BrowserNet {
    allowlist: Vec<String>,
}

impl Net for BrowserNet {
    fn fetch_bytes(&self, url: &str, _method: &str, _body: Option<&[u8]>) -> Result<Vec<u8>> {
        if self.allowlist.iter().any(|prefix| url.starts_with(prefix)) {
            return Err(anyhow!(
                "browser net adapter fetch stub hit for allowed URL {}; host call not wired yet",
                url
            ));
        }
        Err(anyhow!(
            "network blocked by browser adapter policy: {}",
            url
        ))
    }
}

#[derive(Default)]
pub struct BrowserPorts {
    next: Mutex<u16>,
}

impl Ports for BrowserPorts {
    fn reserve(&self, preferred: Option<u16>) -> Result<u16> {
        if let Some(port) = preferred {
            return Ok(port);
        }
        let mut next = self
            .next
            .lock()
            .map_err(|_| anyhow!("failed to lock browser port allocator"))?;
        if *next == 0 {
            *next = 20000;
        }
        let port = *next;
        *next = next.saturating_add(1);
        Ok(port)
    }

    fn release(&self, _port: u16) -> Result<()> {
        Ok(())
    }
}
