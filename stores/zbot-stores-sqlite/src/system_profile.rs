//! System-aware defaults for SQLite pool size and per-connection memory
//! pragmas. Lets the daemon scale gracefully across hardware (Pi Zero →
//! Pi 5 → developer laptop → CI host) without hand-tuning each deploy.
//!
//! Every value can be overridden via env var:
//!
//! - `ZBOT_SQLITE_POOL_MAX`   → u32 pool max_size
//! - `ZBOT_SQLITE_CACHE_KIB`  → u32 cache size in KiB (passed as
//!   `PRAGMA cache_size = -<value>`)
//! - `ZBOT_SQLITE_MMAP_BYTES` → u64 mmap size in bytes
//!
//! When no override is set, defaults are derived from the current
//! machine's CPU count (`std::thread::available_parallelism`) and total
//! RAM (Linux: `/proc/meminfo`; other OSes: assume 4 GiB).

use std::thread::available_parallelism;

const DEFAULT_RAM_KIB: u64 = 4 * 1024 * 1024; // 4 GiB fallback for non-Linux

const POOL_MIN: u32 = 6;
const POOL_MAX: u32 = 32;

const CACHE_MIN_KIB: u32 = 8 * 1024; // 8 MiB
const CACHE_MAX_KIB: u32 = 256 * 1024; // 256 MiB

const MMAP_MIN: u64 = 64 * 1024 * 1024; // 64 MiB
const MMAP_MAX: u64 = 1024 * 1024 * 1024; // 1 GiB

/// Effective SQLite connection-pool max size for this host.
///
/// Formula: `(cores * 2 + 4)` clamped to `[6, 32]`. The +4 baseline is
/// big enough to absorb background workers (sleep, distillation,
/// embedding reindex) without starving HTTP handlers, but small enough
/// to avoid triggering SQLite WAL writer contention on big boxes.
#[must_use]
pub fn pool_max_size() -> u32 {
    if let Some(n) = env_u32("ZBOT_SQLITE_POOL_MAX") {
        return n.clamp(POOL_MIN, POOL_MAX);
    }
    let cores = available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(2) as u32;
    pool_max_size_from_cores(cores)
}

fn pool_max_size_from_cores(cores: u32) -> u32 {
    cores
        .saturating_mul(2)
        .saturating_add(4)
        .clamp(POOL_MIN, POOL_MAX)
}

/// Effective SQLite per-connection cache size in KiB.
///
/// SQLite's `PRAGMA cache_size = -N` allocates `N` KiB. We target
/// roughly 0.4 % of RAM, clamped to `[8 MiB, 256 MiB]`. On a Pi 5 (8 GiB)
/// that's ~32 MiB; on a 32 GiB dev box it's 128 MiB.
#[must_use]
pub fn cache_size_kib() -> u32 {
    if let Some(n) = env_u32("ZBOT_SQLITE_CACHE_KIB") {
        return n.clamp(CACHE_MIN_KIB, CACHE_MAX_KIB);
    }
    cache_size_kib_from_ram(detect_total_ram_kib())
}

fn cache_size_kib_from_ram(ram_kib: u64) -> u32 {
    let target = (ram_kib / 256).min(u64::from(u32::MAX)) as u32;
    target.clamp(CACHE_MIN_KIB, CACHE_MAX_KIB)
}

/// Effective SQLite `mmap_size` in bytes.
///
/// Targets ~3 % of RAM, clamped to `[64 MiB, 1 GiB]`. Generous mmap
/// accelerates random reads (vec0 / FTS5) on large indexes without
/// locking memory the way `cache_size` does.
#[must_use]
pub fn mmap_size_bytes() -> u64 {
    if let Some(n) = env_u64("ZBOT_SQLITE_MMAP_BYTES") {
        return n.clamp(MMAP_MIN, MMAP_MAX);
    }
    mmap_size_from_ram(detect_total_ram_kib())
}

fn mmap_size_from_ram(ram_kib: u64) -> u64 {
    // ram_kib * 1024 / 32  ==  ram_kib * 32  (in bytes)
    let target = ram_kib.saturating_mul(32);
    target.clamp(MMAP_MIN, MMAP_MAX)
}

/// Best-effort detection of total system RAM in KiB.
///
/// Linux: parses `MemTotal:` from `/proc/meminfo`. Other OSes (or any
/// parse failure): falls back to 4 GiB so deployment isn't blocked.
#[must_use]
pub fn detect_total_ram_kib() -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(text) = std::fs::read_to_string("/proc/meminfo") {
            return parse_meminfo_kib(&text).unwrap_or(DEFAULT_RAM_KIB);
        }
    }
    DEFAULT_RAM_KIB
}

fn parse_meminfo_kib(text: &str) -> Option<u64> {
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            // Format: "MemTotal:        8146568 kB"
            return rest
                .split_whitespace()
                .next()
                .and_then(|n| n.parse::<u64>().ok());
        }
    }
    None
}

fn env_u32(key: &str) -> Option<u32> {
    std::env::var(key).ok().and_then(|v| v.trim().parse().ok())
}

fn env_u64(key: &str) -> Option<u64> {
    std::env::var(key).ok().and_then(|v| v.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_size_scales_by_cores_within_bounds() {
        assert_eq!(pool_max_size_from_cores(1), POOL_MIN); // floor
        assert_eq!(pool_max_size_from_cores(2), 8);
        assert_eq!(pool_max_size_from_cores(4), 12);
        assert_eq!(pool_max_size_from_cores(8), 20);
        assert_eq!(pool_max_size_from_cores(64), POOL_MAX); // ceiling
    }

    #[test]
    fn cache_kib_scales_by_ram_within_bounds() {
        // 1 GiB → 4 MiB raw, clamped to floor (8 MiB)
        assert_eq!(cache_size_kib_from_ram(1024 * 1024), CACHE_MIN_KIB);
        // 4 GiB (Pi 4) → 16 MiB
        assert_eq!(cache_size_kib_from_ram(4 * 1024 * 1024), 16 * 1024);
        // 8 GiB (Pi 5) → 32 MiB
        assert_eq!(cache_size_kib_from_ram(8 * 1024 * 1024), 32 * 1024);
        // 64 GiB → would be 256 MiB exactly — at ceiling
        assert_eq!(cache_size_kib_from_ram(64 * 1024 * 1024), CACHE_MAX_KIB);
        // 256 GiB → clamped to 256 MiB
        assert_eq!(cache_size_kib_from_ram(256 * 1024 * 1024), CACHE_MAX_KIB);
    }

    #[test]
    fn mmap_scales_by_ram_within_bounds() {
        // 1 GiB → 32 MiB raw, clamped to 64 MiB floor
        assert_eq!(mmap_size_from_ram(1024 * 1024), MMAP_MIN);
        // 4 GiB (Pi 4) → 128 MiB
        assert_eq!(mmap_size_from_ram(4 * 1024 * 1024), 128 * 1024 * 1024);
        // 8 GiB (Pi 5) → 256 MiB
        assert_eq!(mmap_size_from_ram(8 * 1024 * 1024), 256 * 1024 * 1024);
        // 32 GiB → 1 GiB ceiling
        assert_eq!(mmap_size_from_ram(32 * 1024 * 1024), MMAP_MAX);
    }

    #[test]
    fn parse_meminfo_extracts_kib() {
        let sample = "MemTotal:        8146568 kB\nMemFree:          875800 kB\n";
        assert_eq!(parse_meminfo_kib(sample), Some(8_146_568));
    }

    #[test]
    fn parse_meminfo_returns_none_on_missing_key() {
        assert_eq!(parse_meminfo_kib("MemFree: 100 kB\n"), None);
    }

    #[test]
    fn parse_meminfo_handles_arbitrary_whitespace() {
        let sample = "MemTotal:\t   16384000   kB";
        assert_eq!(parse_meminfo_kib(sample), Some(16_384_000));
    }
}
