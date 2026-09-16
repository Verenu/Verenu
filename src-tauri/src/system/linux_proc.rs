//! /proc parsers used by Linux diagnostics and memory pressure checks.
//!
//! The parsers are OS-independent so they can be unit-tested from fixtures.
//! Live collection walks `/proc` and is compiled only on Linux.

use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProcStat {
    pub pid: u32,
    pub comm: String,
    pub ppid: u32,
    pub utime: u64,
    pub stime: u64,
    pub num_threads: u32,
    pub rss_pages: u64,
}

impl ProcStat {
    pub(crate) fn cpu_ticks(&self) -> u64 {
        self.utime.saturating_add(self.stime)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ProcIo {
    pub read_bytes: u64,
    pub write_bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ProcMeminfo {
    pub total_kb: u64,
    pub available_kb: u64,
}

/// Kernel `stat` fields after the parenthesized comm. Field 1 is pid and
/// field 2 is comm, so `rest[0]` is state (field 3).
pub(crate) fn parse_stat(text: &str) -> Option<ProcStat> {
    let start = text.find('(')?;
    let end = text.rfind(')')?;
    if end <= start {
        return None;
    }
    let pid = text[..start].trim().parse().ok()?;
    let comm = text[start + 1..end].to_string();
    let rest: Vec<&str> = text[end + 1..].split_whitespace().collect();
    Some(ProcStat {
        pid,
        comm,
        ppid: rest.get(1)?.parse().ok()?,
        utime: rest.get(11)?.parse().ok()?,
        stime: rest.get(12)?.parse().ok()?,
        num_threads: rest.get(17)?.parse().ok()?,
        rss_pages: rest.get(21)?.parse().ok()?,
    })
}

pub(crate) fn parse_io(text: &str) -> Option<ProcIo> {
    let mut io = ProcIo::default();
    let mut saw_read = false;
    let mut saw_write = false;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let Ok(value) = value.trim().parse() else {
            continue;
        };
        match key {
            "read_bytes" => {
                io.read_bytes = value;
                saw_read = true;
            }
            "write_bytes" => {
                io.write_bytes = value;
                saw_write = true;
            }
            _ => {}
        }
    }
    (saw_read && saw_write).then_some(io)
}

pub(crate) fn parse_status_kb(text: &str, key: &str) -> Option<u64> {
    let prefix = format!("{key}:");
    let line = text
        .lines()
        .find(|line| line.trim_start().starts_with(&prefix))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

/// `smaps_rollup` keys are the same `Name:   123 kB` shape as `/proc/status`.
pub(crate) fn parse_smaps_kb(text: &str, key: &str) -> Option<u64> {
    parse_status_kb(text, key)
}

pub(crate) fn parse_status_threads(text: &str) -> Option<u32> {
    let line = text
        .lines()
        .find(|line| line.trim_start().starts_with("Threads:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

pub(crate) fn parse_meminfo(text: &str) -> Option<ProcMeminfo> {
    let mut info = ProcMeminfo::default();
    for line in text.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let Some(kb) = rest.split_whitespace().next().and_then(|value| value.parse().ok()) else {
            continue;
        };
        match key {
            "MemTotal" => info.total_kb = kb,
            "MemAvailable" => info.available_kb = kb,
            _ => {}
        }
    }
    (info.total_kb > 0 && info.available_kb > 0).then_some(info)
}

#[cfg(target_os = "linux")]
pub(crate) fn process_tree_memory_bytes(root: u32) -> Option<u64> {
    let tree = process_tree(root)?;
    let mut total = 0u64;
    for pid in tree {
        if let Some(kb) = process_accounted_kb(pid) {
            total = total.saturating_add(kb.saturating_mul(1024));
        }
    }
    (total > 0).then_some(total)
}

/// Prefer PSS so shared WebKit/GPU mappings are not counted once per process.
/// Fall back to anonymous RSS, then total RSS, if rollup is unreadable.
#[cfg(target_os = "linux")]
fn process_accounted_kb(pid: u32) -> Option<u64> {
    if let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")) {
        if let Some(kb) = parse_smaps_kb(&text, "Pss") {
            return Some(kb);
        }
    }
    read_status_kb(pid, "RssAnon").or_else(|| read_status_kb(pid, "VmRSS"))
}

#[cfg(target_os = "linux")]
pub(crate) fn system_meminfo() -> Option<ProcMeminfo> {
    parse_meminfo(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

#[cfg(target_os = "linux")]
pub(crate) fn fill_resource_snapshot(
    sample: &mut crate::system::diagnostics::ResourceSnapshot,
    root: u32,
) {
    use std::sync::{Mutex, OnceLock};
    use std::time::Instant;

    type PreviousCpu = Option<(Instant, HashMap<u32, u64>)>;
    static PREVIOUS_CPU: OnceLock<Mutex<PreviousCpu>> = OnceLock::new();
    static PREVIOUS_IO: OnceLock<Mutex<Option<(Instant, u64, u64)>>> = OnceLock::new();

    let Some(tree) = process_tree(root) else {
        return;
    };
    sample.process_count = Some(tree.len() as u32);

    let mut children = Vec::new();
    let mut thread_count = 0u32;
    let mut read_total = 0u64;
    let mut write_total = 0u64;
    let mut current_cpu = HashMap::with_capacity(tree.len());

    for pid in &tree {
        let Ok(stat_text) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        let Some(stat) = parse_stat(&stat_text) else {
            continue;
        };
        current_cpu.insert(*pid, stat.cpu_ticks());
        thread_count = thread_count.saturating_add(stat.num_threads.max(1));
        if let Some(io) = std::fs::read_to_string(format!("/proc/{pid}/io"))
            .ok()
            .and_then(|text| parse_io(&text))
        {
            read_total = read_total.saturating_add(io.read_bytes);
            write_total = write_total.saturating_add(io.write_bytes);
        }
        if *pid != root && children.len() < 16 {
            children.push(crate::system::diagnostics::ChildProcessSnapshot {
                label: stat.comm,
                pid: Some(*pid),
                resident_bytes: process_accounted_kb(*pid).map(|kb| kb.saturating_mul(1024)),
                thread_count: Some(stat.num_threads.max(1)),
                ..Default::default()
            });
        }
    }

    sample.thread_count = (thread_count > 0).then_some(thread_count);
    sample.child_processes = children;
    sample.read_bytes_total = Some(read_total);
    sample.write_bytes_total = Some(write_total);

    let now = Instant::now();
    if let Ok(mut previous) = PREVIOUS_IO.get_or_init(|| Mutex::new(None)).lock() {
        if let Some((at, old_read, old_write)) = previous.replace((now, read_total, write_total)) {
            let seconds = now.duration_since(at).as_secs_f64();
            if seconds > 0.0 {
                sample.read_bytes_per_sec =
                    Some(read_total.saturating_sub(old_read) as f64 / seconds);
                sample.write_bytes_per_sec =
                    Some(write_total.saturating_sub(old_write) as f64 / seconds);
            }
        }
    }

    let ticks_per_sec = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f64;
    let logical_cpus = std::thread::available_parallelism()
        .map(|n| n.get() as f64)
        .unwrap_or(1.0)
        .max(1.0);
    if let Ok(mut previous) = PREVIOUS_CPU.get_or_init(|| Mutex::new(None)).lock() {
        if let Some((at, old_cpu)) = previous.as_ref() {
            let seconds = now.duration_since(*at).as_secs_f64();
            if seconds > 0.0 {
                let cpu_ticks = current_cpu
                    .iter()
                    .map(|(pid, ticks)| ticks.saturating_sub(*old_cpu.get(pid).unwrap_or(&0)))
                    .sum::<u64>() as f64;
                sample.cpu_percent = Some(
                    ((cpu_ticks / (seconds * ticks_per_sec * logical_cpus)) * 100.0)
                        .clamp(0.0, 100.0),
                );
            }
        }
        *previous = Some((now, current_cpu));
    }
}

#[cfg(target_os = "linux")]
fn read_status_kb(pid: u32, key: &str) -> Option<u64> {
    parse_status_kb(
        &std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?,
        key,
    )
}

#[cfg(target_os = "linux")]
fn process_tree(root: u32) -> Option<Vec<u32>> {
    let mut children_of: HashMap<u32, Vec<u32>> = HashMap::new();
    let proc = std::fs::read_dir("/proc").ok()?;
    for entry in proc.flatten() {
        let name = entry.file_name();
        let Ok(pid) = name.to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(stat_text) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        let Some(stat) = parse_stat(&stat_text) else {
            continue;
        };
        if stat.ppid != 0 && stat.ppid != pid {
            children_of.entry(stat.ppid).or_default().push(pid);
        }
    }

    let mut tree = Vec::new();
    let mut seen = HashSet::new();
    let mut queue = VecDeque::new();
    queue.push_back(root);
    while let Some(pid) = queue.pop_front() {
        if !seen.insert(pid) {
            continue;
        }
        tree.push(pid);
        if tree.len() >= 17 {
            break;
        }
        if let Some(children) = children_of.get(&pid) {
            for child in children {
                queue.push_back(*child);
            }
        }
    }
    (!tree.is_empty()).then_some(tree)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAT_STAT: &str = "303412 (cat) R 303338 303338 303338 0 -1 4194304 94 0 0 0 0 0 0 0 20 0 1 0 504646 6385664 545 18446744073709551615 94329894354944 94329894381617 140727249584816 0 0 0 0 0 0 0 0 0 17 3 0 0 0 0 0 94329894398448 94329894400228 94330731192320 140727249587712 140727249587732 140727249587732 140727249596395 0\n";

    #[test]
    fn parse_stat_reads_pid_ppid_cpu_threads_and_rss() {
        let stat = parse_stat(CAT_STAT).expect("stat");
        assert_eq!(stat.pid, 303412);
        assert_eq!(stat.comm, "cat");
        assert_eq!(stat.ppid, 303338);
        assert_eq!(stat.utime, 0);
        assert_eq!(stat.stime, 0);
        assert_eq!(stat.num_threads, 1);
        assert_eq!(stat.rss_pages, 545);
        assert_eq!(stat.cpu_ticks(), 0);
    }

    #[test]
    fn parse_stat_keeps_spaces_inside_comm() {
        let stat = parse_stat("12 (verenu helper) S 1 1 1 0 -1 0 0 0 0 0 8 4 0 0 20 0 12 0 1 0 99\n")
            .expect("stat");
        assert_eq!(stat.pid, 12);
        assert_eq!(stat.comm, "verenu helper");
        assert_eq!(stat.ppid, 1);
        assert_eq!(stat.utime, 8);
        assert_eq!(stat.stime, 4);
        assert_eq!(stat.num_threads, 12);
        assert_eq!(stat.rss_pages, 99);
        assert_eq!(stat.cpu_ticks(), 12);
    }

    #[test]
    fn parse_io_uses_block_io_not_syscall_char_counts() {
        let io = parse_io(
            "rchar: 2884\nwchar: 10\nsyscr: 6\nsyscw: 0\nread_bytes: 4096\nwrite_bytes: 8192\ncancelled_write_bytes: 0\n",
        )
        .expect("io");
        assert_eq!(io.read_bytes, 4096);
        assert_eq!(io.write_bytes, 8192);
    }

    #[test]
    fn parse_smaps_rollup_prefers_pss() {
        let text = "Rss:              285960 kB\nPss:              161518 kB\nPrivate_Dirty:     78084 kB\n";
        assert_eq!(parse_smaps_kb(text, "Pss"), Some(161518));
        assert_eq!(parse_smaps_kb(text, "Rss"), Some(285960));
    }

    #[test]
    fn parse_status_and_meminfo_read_kb_fields() {
        assert_eq!(
            parse_status_kb("Name:\tverenu\nVmRSS:\t6240 kB\nThreads:\t8\n", "VmRSS"),
            Some(6240)
        );
        assert_eq!(
            parse_status_threads("Name:\tverenu\nVmRSS:\t6240 kB\nThreads:\t8\n"),
            Some(8)
        );
        let mem = parse_meminfo("MemTotal:       31991180 kB\nMemFree: 1000 kB\nMemAvailable:   24824028 kB\n")
            .expect("meminfo");
        assert_eq!(mem.total_kb, 31_991_180);
        assert_eq!(mem.available_kb, 24_824_028);
    }

    #[test]
    fn parse_rejects_truncated_records() {
        assert!(parse_stat("12 (nope)").is_none());
        assert!(parse_io("rchar: 1\n").is_none());
        assert!(parse_meminfo("MemTotal: 1 kB\n").is_none());
    }
}
