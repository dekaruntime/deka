use std::cell::Cell;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use super::config::PoolConfig;

pub(super) const ID_ALPHABET: [char; 62] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I',
    'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', 'a', 'b',
    'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u',
    'v', 'w', 'x', 'y', 'z',
];

pub(super) static POOL_IDS: AtomicU64 = AtomicU64::new(1);
pub(super) const REQUEST_BATCH_MAX: usize = 8;
pub(super) static PERF_COUNT: AtomicU64 = AtomicU64::new(0);
pub(super) static PERF_QUEUE_TOTAL_MS: AtomicU64 = AtomicU64::new(0);
pub(super) static PERF_WARM_TOTAL_MS: AtomicU64 = AtomicU64::new(0);
pub(super) static PERF_EXEC_TOTAL_MS: AtomicU64 = AtomicU64::new(0);
pub(super) static PERF_EVENT_TOTAL_MS: AtomicU64 = AtomicU64::new(0);
pub(super) static PERF_RESULT_TOTAL_MS: AtomicU64 = AtomicU64::new(0);
pub(super) static PERF_TOTAL_MS: AtomicU64 = AtomicU64::new(0);

thread_local! {
    pub(super) static CURRENT_WORKER_ID: Cell<Option<usize>> = Cell::new(None);
    pub(super) static CURRENT_POOL_ID: Cell<Option<u64>> = Cell::new(None);
}

// ========== OS-level Thread CPU Time ==========

/// Get CPU time consumed by current thread
#[cfg(target_os = "linux")]
pub(super) fn get_thread_cpu_time() -> Duration {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe {
        libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts);
    }
    Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

/// Get CPU time consumed by current thread (macOS)
#[cfg(target_os = "macos")]
pub(super) fn get_thread_cpu_time() -> Duration {
    use libc::{THREAD_BASIC_INFO, thread_basic_info, thread_info};
    use mach2::mach_init::mach_thread_self;

    unsafe {
        let mut info = std::mem::zeroed::<thread_basic_info>();
        let mut count =
            (std::mem::size_of::<thread_basic_info>() / std::mem::size_of::<libc::c_int>()) as u32;

        let kr = thread_info(
            mach_thread_self(),
            THREAD_BASIC_INFO as u32,
            &mut info as *mut _ as *mut _,
            &mut count,
        );

        if kr == 0 {
            let user_secs = info.user_time.seconds as u64;
            let user_usecs = info.user_time.microseconds as u32;
            let sys_secs = info.system_time.seconds as u64;
            let sys_usecs = info.system_time.microseconds as u32;

            let user = Duration::new(user_secs, user_usecs * 1000);
            let sys = Duration::new(sys_secs, sys_usecs * 1000);

            user + sys
        } else {
            Duration::ZERO
        }
    }
}

/// Fallback for unsupported platforms
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn get_thread_cpu_time() -> Duration {
    Duration::ZERO
}

pub(super) fn perf_profile_enabled(config: &PoolConfig) -> bool {
    config.perf_profile
}
