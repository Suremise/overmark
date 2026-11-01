//! This process: its memory and CPU use, and giving memory back.

use windows::Win32::Foundation::{FILETIME, HANDLE};
use windows::Win32::System::ProcessStatus::{
    EmptyWorkingSet, GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// A process's memory, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Memory {
    /// Its pages in RAM now (what Task Manager's details call the working set).
    pub working_set: u64,
    /// The memory it has committed for itself (private bytes).
    pub private: u64,
}

/// A process to measure: this one, or another by its ID.
pub struct Process(HANDLE, bool);

impl Process {
    pub fn current() -> Process {
        // SAFETY: a pseudo-handle, never closed.
        Process(unsafe { GetCurrentProcess() }, false)
    }

    /// Another process, for reading its memory and CPU times.
    pub fn open(pid: u32) -> windows::core::Result<Process> {
        // SAFETY: opens a handle we close in Drop.
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
            .map(|h| Process(h, true))
    }

    pub fn memory(&self) -> Memory {
        let mut c = PROCESS_MEMORY_COUNTERS_EX {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            ..Default::default()
        };
        // SAFETY: the counters struct is ours, with its size set.
        let ok = unsafe {
            GetProcessMemoryInfo(
                self.0,
                &mut c as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS,
                c.cb,
            )
        };
        if ok.is_err() {
            return Memory::default();
        }
        Memory {
            working_set: c.WorkingSetSize as u64,
            private: c.PrivateUsage as u64,
        }
    }

    /// The CPU time it has used, kernel and user, in seconds.
    pub fn cpu_seconds(&self) -> f64 {
        let (mut a, mut b, mut k, mut u) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        // SAFETY: four FILETIMEs we own.
        if unsafe { GetProcessTimes(self.0, &mut a, &mut b, &mut k, &mut u) }.is_err() {
            return 0.0;
        }
        let t =
            |f: FILETIME| (u64::from(f.dwHighDateTime) << 32 | u64::from(f.dwLowDateTime)) as f64;
        (t(k) + t(u)) / 10_000_000.0
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if self.1 {
            // SAFETY: a handle we opened, closed once.
            let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.0) };
        }
    }
}

/// Hands this process's pages back to Windows (they come back as they're used): after the
/// overlay closes, so the idle footprint is the tray's.
pub fn trim_working_set() {
    // SAFETY: our own process's pseudo-handle.
    let _ = unsafe { EmptyWorkingSet(GetCurrentProcess()) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_this_process() {
        let p = Process::current();
        let m = p.memory();
        assert!(m.working_set > 0 && m.private > 0, "{m:?}");
        // CPU time is counted in clock ticks (about 15.6 ms): work until it shows, with a
        // generous bound for a hung run, rather than a fixed amount of work.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut x = 0u64;
        while p.cpu_seconds() <= 0.0 {
            assert!(std::time::Instant::now() < deadline, "no CPU time counted");
            for i in 0..1_000_000u64 {
                x = std::hint::black_box(x.wrapping_add(i * i));
            }
        }
        assert!(p.cpu_seconds() > 0.0, "{x}");
    }
}
