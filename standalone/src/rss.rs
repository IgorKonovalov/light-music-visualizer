//! Dependency-free per-OS resident-set-size (working-set) read for the
//! standalone's diagnostics log (Plan 0011).
//!
//! RSS is deliberately NOT in the C ABI (ADR-0008): it is the host process's own
//! working set, so each shell reads its own. This one is the standalone's. Raw
//! OS calls only — no new crate (NFR 4); on Windows it reuses the `windows`
//! bindings the capture layer already pulls in.

/// The process's current resident set / working set in bytes, or `None` if the
/// OS query fails or the platform is unsupported.
#[cfg(windows)]
pub fn current_rss_bytes() -> Option<u64> {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;

    let mut counters = PROCESS_MEMORY_COUNTERS::default();
    let cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    // SAFETY: `counters` is a zeroed POD the call fills; `GetCurrentProcess`
    // returns a pseudo-handle that needs no close.
    let ok = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, cb) };
    ok.is_ok().then_some(counters.WorkingSetSize as u64)
}

/// macOS working set via mach `task_info(MACH_TASK_BASIC_INFO)`. Raw mach
/// bindings so we add no dependency; unvalidated pending a Mac (the standing
/// Plan 0001 carry-forward — macOS is not validated this plan).
#[cfg(target_os = "macos")]
pub fn current_rss_bytes() -> Option<u64> {
    const MACH_TASK_BASIC_INFO: u32 = 20;

    #[repr(C)]
    #[derive(Default)]
    struct TimeValue {
        seconds: i32,
        microseconds: i32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct MachTaskBasicInfo {
        virtual_size: u64,
        resident_size: u64,
        resident_size_max: u64,
        user_time: TimeValue,
        system_time: TimeValue,
        policy: i32,
        suspend_count: i32,
    }

    unsafe extern "C" {
        static mach_task_self_: u32;
        fn task_info(
            target_task: u32,
            flavor: u32,
            task_info_out: *mut i32,
            task_info_count: *mut u32,
        ) -> i32;
    }

    let mut info = MachTaskBasicInfo::default();
    let mut count = (std::mem::size_of::<MachTaskBasicInfo>() / std::mem::size_of::<u32>()) as u32;
    // SAFETY: `task_info` writes `count` u32 words into `info`, which is sized to
    // MACH_TASK_BASIC_INFO's word count; `mach_task_self_` is the current task.
    let kr = unsafe {
        task_info(
            mach_task_self_,
            MACH_TASK_BASIC_INFO,
            std::ptr::from_mut(&mut info).cast::<i32>(),
            &mut count,
        )
    };
    (kr == 0).then_some(info.resident_size)
}

/// Other platforms: `/proc/self/statm` (page counts; the 2nd is resident).
/// Linux page size is 4096 on the targets we build for; hardcoded to stay
/// libc-free.
#[cfg(not(any(windows, target_os = "macos")))]
pub fn current_rss_bytes() -> Option<u64> {
    const PAGE_SIZE: u64 = 4096;
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let resident_pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(resident_pages * PAGE_SIZE)
}
