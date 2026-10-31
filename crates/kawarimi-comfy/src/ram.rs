//! System memory around a render. ComfyUI's Python takes 21–24 GB of RAM for Wan on an 8 GB card (docs/RENDER.md),
//! so a render is checked against the machine's memory before it is queued, and watched while ComfyUI runs: when the
//! commit headroom (what can still be allocated, RAM and page file together) falls under [`FLOOR`], ComfyUI is
//! stopped before the machine freezes, and the job fails with the reason.
//!
//! Free physical RAM is not the signal: it sits near zero through every render measured on 32 GB, and those finished,
//! because the system paged.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kawarimi_assets::manifest;
use serde::{Deserialize, Serialize};

use crate::variant::Variant;

pub const GB: u64 = 1 << 30;
/// Under this much commit headroom other programs' allocations start failing and the desktop stalls.
pub const FLOOR: u64 = 3 * GB / 2;
const TICK: Duration = Duration::from_millis(250);
/// ComfyUI with Q3_K_M at one 77-frame 848×480 window: 20.5–23.6 GB measured (docs/RENDER.md), rounded up.
const MEASURED_NEED: u64 = 24 * GB;

/// The machine's memory now, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Memory {
    pub total: u64,
    /// Physical RAM free for new use (Windows' available, Linux's `MemAvailable`).
    pub free: u64,
    /// What can still be committed: Windows' available page file (RAM and page file) plus the room the page file
    /// still has to grow, Linux's `MemAvailable + SwapFree`.
    pub headroom: u64,
}

impl Memory {
    #[cfg(windows)]
    pub fn read() -> Option<Self> {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        // SAFETY: a plain Win32 call on a struct we own, with its length set as the call requires.
        let status = unsafe {
            let mut status: MEMORYSTATUSEX = std::mem::zeroed();
            status.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
            (GlobalMemoryStatusEx(&mut status) != 0).then_some(status)?
        };
        // The commit limit is RAM plus the page files as they are now.
        let page_file = status.ullTotalPageFile.saturating_sub(status.ullTotalPhys);
        let grows = paging_files().map_or(0, |files| growth(&files, status.ullTotalPhys, page_file, disk));
        Some(Self { total: status.ullTotalPhys, free: status.ullAvailPhys, headroom: status.ullAvailPageFile + grows })
    }

    #[cfg(not(windows))]
    pub fn read() -> Option<Self> {
        meminfo(&std::fs::read_to_string("/proc/meminfo").ok()?)
    }
}

/// How much more the first page file can grow: Windows grows a system-managed one on demand (renders on 32 GB
/// finished that way) up to the larger of 3 × RAM and 4 GB, at most an eighth of its volume; a custom one up to its
/// maximum. `paging_files` is the registry's `PagingFiles` (`?:\pagefile.sys`, `C:\pagefile.sys 0 0`, or with sizes
/// in MB); `disk` gives a drive's free and total bytes.
#[cfg_attr(not(windows), allow(dead_code))]
fn growth(paging_files: &str, ram: u64, page_file: u64, disk: impl Fn(&str) -> Option<(u64, u64)>) -> u64 {
    let Some(entry) = paging_files.split(['\0', '\n']).map(str::trim).find(|e| !e.is_empty()) else { return 0 };
    let mut parts = entry.split_whitespace();
    let path = parts.next().unwrap_or_default();
    let sizes: Vec<u64> = parts.filter_map(|p| p.parse().ok()).collect();
    let drive = match path.get(..2) {
        Some("?:") => std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into()),
        Some(d) => d.to_string(),
        None => return 0,
    };
    let Some((free, volume)) = disk(&drive) else { return 0 };
    let cap = match sizes[..] {
        [] | [0, 0] => (3 * ram).max(4 * GB).min(volume / 8),
        [_, max] => max << 20,
        _ => return 0,
    };
    cap.saturating_sub(page_file).min(free)
}

#[cfg(windows)]
fn paging_files() -> Option<String> {
    use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_MULTI_SZ, RegGetValueW};
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let key = wide(r"SYSTEM\CurrentControlSet\Control\Session Manager\Memory Management");
    let value = wide("PagingFiles");
    let mut buffer = [0u16; 1024];
    let mut bytes = size_of_val(&buffer) as u32;
    // SAFETY: a registry read into a buffer whose size we pass.
    let error = unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), value.as_ptr(), RRF_RT_REG_MULTI_SZ, std::ptr::null_mut(), buffer.as_mut_ptr().cast(), &mut bytes)
    };
    (error == 0).then(|| String::from_utf16_lossy(&buffer[..bytes as usize / 2]))
}

/// A drive's (`C:`) free and total bytes.
#[cfg(windows)]
fn disk(drive: &str) -> Option<(u64, u64)> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let root: Vec<u16> = format!("{drive}\\").encode_utf16().chain([0]).collect();
    let (mut free, mut total) = (0u64, 0u64);
    // SAFETY: a plain Win32 call with out-pointers to locals.
    let ok = unsafe { GetDiskFreeSpaceExW(root.as_ptr(), &mut free, &mut total, std::ptr::null_mut()) };
    (ok != 0).then_some((free, total))
}

/// `/proc/meminfo`'s numbers, in kB.
#[cfg_attr(windows, allow(dead_code))]
fn meminfo(text: &str) -> Option<Memory> {
    let field = |name: &str| -> Option<u64> {
        let line = text.lines().find(|l| l.starts_with(name) && l[name.len()..].starts_with(':'))?;
        Some(line[name.len() + 1..].trim().trim_end_matches("kB").trim().parse::<u64>().ok()? * 1024)
    };
    let free = field("MemAvailable")?;
    Some(Memory { total: field("MemTotal")?, free, headroom: free + field("SwapFree").unwrap_or(0) })
}

/// The RAM ComfyUI takes to render with `variant`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Need {
    pub bytes: u64,
    /// False for a variant never measured: Q3_K_M's measurement plus the difference in weights.
    pub measured: bool,
}

pub fn need(variant: &Variant) -> Need {
    let q3 = &manifest::WAN_ANIMATE_Q3;
    let more = variant.asset.download_size().saturating_sub(q3.download_size());
    Need { bytes: MEASURED_NEED + more, measured: variant.asset.id == q3.id }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    Fits,
    /// Physical RAM is short but RAM and page file together hold it: it runs, slower, and other programs may stall.
    Pages,
    /// Not even RAM and page file together hold it above the floor.
    Short,
}

/// A render's memory against the machine's, before it is queued.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Fit {
    pub verdict: Verdict,
    pub need: u64,
    pub measured: bool,
    pub free: u64,
    pub headroom: u64,
    /// What to tell the person; none when it fits.
    pub message: Option<String>,
}

/// `held` is what the ComfyUI that will render already has (its models from the last render), counted as free.
pub fn fit(need: Need, memory: Memory, held: u64) -> Fit {
    let free = memory.free + held;
    let headroom = memory.headroom + held;
    let verdict = if need.bytes + FLOOR > headroom {
        Verdict::Short
    } else if need.bytes > free {
        Verdict::Pages
    } else {
        Verdict::Fits
    };
    let about = format!("about {}{}", gb(need.bytes), if need.measured { "" } else { " (an estimate)" });
    let message = match verdict {
        Verdict::Fits => None,
        Verdict::Pages => Some(format!(
            "This render needs {about} of RAM and {} is free, so the system will page: it runs, slower, and other programs may stall.",
            gb(free)
        )),
        Verdict::Short => Some(format!(
            "Not enough memory: this render needs {about} of RAM, and RAM and page file together have {} free. Close other programs or make the page file larger.",
            gb(headroom)
        )),
    };
    Fit { verdict, need: need.bytes, measured: need.measured, free, headroom, message }
}

/// Whether the watchdog stops ComfyUI.
pub fn crossed(memory: Memory) -> bool {
    memory.headroom < FLOOR
}

/// Why a render was stopped at the floor.
pub fn ran_out(memory: Memory) -> String {
    format!(
        "RAM ran out: {} left of RAM and page file together, so ComfyUI was stopped before the computer froze. Close other programs or make the page file larger, then retry.",
        gb(memory.headroom)
    )
}

fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / GB as f64)
}

/// A render's memory, as the watchdog saw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    /// ComfyUI's process tree, working sets summed.
    pub peak_rss: u64,
    pub min_free: u64,
    pub min_headroom: u64,
}

impl Stats {
    fn of(rss: u64, memory: Memory) -> Self {
        Self { peak_rss: rss, min_free: memory.free, min_headroom: memory.headroom }
    }

    pub fn merge(self, other: Self) -> Self {
        Self {
            peak_rss: self.peak_rss.max(other.peak_rss),
            min_free: self.min_free.min(other.min_free),
            min_headroom: self.min_headroom.min(other.min_headroom),
        }
    }

    pub fn merged(a: Option<Self>, b: Option<Self>) -> Option<Self> {
        match (a, b) {
            (Some(a), Some(b)) => Some(a.merge(b)),
            (a, b) => a.or(b),
        }
    }
}

/// The watchdog of one ComfyUI: what it saw, and why it stopped it.
#[derive(Debug, Default)]
pub struct Guard {
    state: Mutex<State>,
    stopped: AtomicBool,
}

#[derive(Debug, Default)]
struct State {
    stats: Option<Stats>,
    rss: u64,
    tripped: Option<String>,
}

impl Guard {
    /// Since the last [`Guard::reset`].
    pub fn stats(&self) -> Option<Stats> {
        self.state.lock().unwrap().stats
    }

    /// Starts the stats afresh, for a new render.
    pub fn reset(&self) {
        self.state.lock().unwrap().stats = None;
    }

    /// ComfyUI's RAM at the last look.
    pub fn held(&self) -> u64 {
        self.state.lock().unwrap().rss
    }

    /// Why the watchdog stopped ComfyUI, if it did.
    pub fn tripped(&self) -> Option<String> {
        self.state.lock().unwrap().tripped.clone()
    }

    /// Ends the watch; ComfyUI is being stopped on purpose.
    pub(crate) fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }

    fn look(&self, rss: u64, memory: Memory) -> bool {
        let mut state = self.state.lock().unwrap();
        state.rss = rss;
        state.stats = Stats::merged(state.stats, Some(Stats::of(rss, memory)));
        if crossed(memory) && state.tripped.is_none() {
            state.tripped = Some(ran_out(memory));
            return true;
        }
        false
    }
}

/// Watches the machine's memory and `rss` (ComfyUI's tree) every [`TICK`] until [`Guard::stop`]; calls `kill` once
/// at the floor.
pub(crate) fn watch(rss: impl Fn() -> u64 + Send + 'static, kill: impl FnOnce() + Send + 'static) -> Arc<Guard> {
    let guard = Arc::new(Guard::default());
    let watching = guard.clone();
    std::thread::spawn(move || {
        let mut kill = Some(kill);
        while !watching.stopped.load(Ordering::SeqCst) {
            if let Some(memory) = Memory::read()
                && watching.look(rss(), memory)
                && let Some(kill) = kill.take()
            {
                kill();
            }
            std::thread::sleep(TICK);
        }
    });
    guard
}

/// Working sets of a Windows job's processes, summed.
#[cfg(windows)]
pub(crate) fn job_rss(job: std::os::windows::io::RawHandle) -> u64 {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::{JOBOBJECT_BASIC_PROCESS_ID_LIST, JobObjectBasicProcessIdList, QueryInformationJobObject};
    use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    // The list header is two u32s, then the ids; ComfyUI starts a handful of processes.
    let mut buffer = [0usize; 66];
    // SAFETY: Win32 calls on a live job handle and a buffer the size we pass; process handles are closed here.
    unsafe {
        let ok = QueryInformationJobObject(job, JobObjectBasicProcessIdList, buffer.as_mut_ptr().cast(), size_of_val(&buffer) as u32, std::ptr::null_mut());
        if ok == 0 {
            return 0;
        }
        let list = &*buffer.as_ptr().cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>();
        let ids = std::slice::from_raw_parts(list.ProcessIdList.as_ptr(), list.NumberOfProcessIdsInList as usize);
        let mut total = 0;
        for &pid in ids {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid as u32);
            if process.is_null() {
                continue;
            }
            let mut counters: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
            if K32GetProcessMemoryInfo(process, &mut counters, size_of::<PROCESS_MEMORY_COUNTERS>() as u32) != 0 {
                total += counters.WorkingSetSize as u64;
            }
            CloseHandle(process);
        }
        total
    }
}

/// Resident memory of a process group's processes, summed.
#[cfg(unix)]
pub(crate) fn group_rss(group: u32) -> u64 {
    // note: assumes 4 KiB pages (x86-64 Linux); a 64 KiB-page kernel would read 16× low.
    const PAGE: u64 = 4096;
    let entries = std::fs::read_dir("/proc").into_iter().flatten().flatten();
    entries
        .filter_map(|e| std::fs::read_to_string(e.path().join("stat")).ok())
        .filter_map(|stat| {
            // After the command's closing parenthesis: state, ppid, pgrp, … rss is the 22nd field from there.
            let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
            (fields.get(2)?.parse::<u32>().ok()? == group).then(|| fields.get(21)?.parse::<u64>().ok()).flatten()
        })
        .map(|pages| pages * PAGE)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VARIANTS;

    fn memory(free_gb: f64, headroom_gb: f64) -> Memory {
        let b = |g: f64| (g * GB as f64) as u64;
        Memory { total: 32 * GB, free: b(free_gb), headroom: b(headroom_gb) }
    }

    const Q3: Need = Need { bytes: 24 * GB, measured: true };

    #[test]
    fn the_floor_is_on_commit_headroom_not_free_ram() {
        // Every measured 32 GB render ran with free RAM at 2–43 MB, and finished.
        assert!(!crossed(memory(0.002, 6.0)));
        assert!(!crossed(memory(0.0, 1.6)));
        assert!(crossed(memory(0.0, 1.4)));
        assert!(crossed(memory(8.0, 1.0)));
        assert!(ran_out(memory(0.0, 1.2)).starts_with("RAM ran out: 1.2 GB left"));
    }

    #[test]
    fn a_render_fits_pages_or_is_refused() {
        assert_eq!(fit(Q3, memory(26.0, 40.0), 0).verdict, Verdict::Fits);
        // The measured machine: ~18 GB taken by other programs, a page file the size of RAM.
        let pages = fit(Q3, memory(14.0, 40.0), 0);
        assert_eq!(pages.verdict, Verdict::Pages);
        assert!(pages.message.unwrap().contains("needs about 24.0 GB of RAM and 14.0 GB is free"));
        let short = fit(Q3, memory(14.0, 20.0), 0);
        assert_eq!(short.verdict, Verdict::Short);
        assert!(short.message.unwrap().contains("together have 20.0 GB free"));
        // 24 GB plus the floor is the line.
        assert_eq!(fit(Q3, memory(4.0, 25.4), 0).verdict, Verdict::Short);
        assert_eq!(fit(Q3, memory(4.0, 25.6), 0).verdict, Verdict::Pages);
    }

    #[test]
    fn what_comfy_already_holds_counts_as_free() {
        // A ComfyUI kept from the last render holds its 22 GB; the next render reuses it.
        assert_eq!(fit(Q3, memory(4.0, 10.0), 22 * GB).verdict, Verdict::Fits);
    }

    #[test]
    fn only_q3_is_measured_and_bigger_weights_need_more() {
        let q3 = need(Variant::find(manifest::WAN_ANIMATE_Q3.id).unwrap());
        assert_eq!(q3, Q3);
        for variant in VARIANTS {
            let n = need(variant);
            assert_eq!(n.measured, variant.asset.id == manifest::WAN_ANIMATE_Q3.id);
            assert!(n.bytes >= q3.bytes);
        }
        let estimate = fit(need(Variant::find(manifest::WAN_ANIMATE_FP8.id).unwrap()), memory(10.0, 40.0), 0);
        assert!(estimate.message.unwrap().contains("(an estimate)"));
    }

    #[test]
    fn the_guard_trips_once_and_keeps_extremes() {
        let guard = Guard::default();
        assert!(!guard.look(20 * GB, memory(0.04, 6.0)));
        assert!(!guard.look(22 * GB, memory(0.5, 3.0)));
        assert!(guard.look(21 * GB, memory(0.01, 1.2)));
        assert!(!guard.look(21 * GB, memory(0.01, 1.0)), "kills once");
        let stats = guard.stats().unwrap();
        assert_eq!(stats.peak_rss, 22 * GB);
        assert_eq!(stats.min_free, memory(0.01, 0.0).free);
        assert_eq!(stats.min_headroom, GB);
        assert!(guard.tripped().unwrap().contains("1.2 GB left"));
        guard.reset();
        assert_eq!(guard.stats(), None);
        assert_eq!(guard.held(), 21 * GB);
    }

    #[test]
    fn a_system_managed_page_file_counts_its_room_to_grow() {
        let disk = |free: u64| move |_: &str| Some((free * GB, 1000 * GB));
        // 32 GB of RAM, a 14 GB page file now: it may grow to 96 GB, as far as the disk's 50 GB free allow.
        assert_eq!(growth("?:\\pagefile.sys\0\0", 32 * GB, 14 * GB, disk(50)), 50 * GB);
        assert_eq!(growth("C:\\pagefile.sys 0 0", 32 * GB, 14 * GB, disk(500)), 82 * GB);
        // A custom 16 GB maximum, and a fixed size.
        assert_eq!(growth("D:\\pagefile.sys 4096 16384", 32 * GB, 14 * GB, disk(500)), 2 * GB);
        assert_eq!(growth("C:\\pagefile.sys 14336 14336", 32 * GB, 14 * GB, disk(500)), 0);
        // An eighth of a small volume caps it; no page file, no growth.
        assert_eq!(growth("?:\\pagefile.sys", 32 * GB, 10 * GB, |_| Some((200 * GB, 256 * GB))), 22 * GB);
        assert_eq!(growth("", 32 * GB, 0, disk(500)), 0);
    }

    #[test]
    fn meminfo_is_read_in_kb() {
        let text = "MemTotal:       32768000 kB\nMemFree:          100000 kB\nMemAvailable:    4000000 kB\nSwapTotal: 8000000 kB\nSwapFree:        2000000 kB\n";
        let m = meminfo(text).unwrap();
        assert_eq!((m.total, m.free, m.headroom), (32768000 * 1024, 4000000 * 1024, 6000000 * 1024));
    }

    #[test]
    fn this_machine_reports_its_memory() {
        let m = Memory::read().unwrap();
        eprintln!("{m:?}");
        assert!(m.total > GB && m.free <= m.total && m.headroom > 0);
    }
}
