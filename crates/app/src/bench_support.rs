//! Portable input loading and latency summaries for the model comparison.

use manga_core::lang::Lang;
use std::path::{Path, PathBuf};

pub struct Corpus {
    pub path: PathBuf,
    pub lines: Vec<String>,
}

pub fn load_corpus(install: &Path, data: &Path, lang: Lang) -> std::io::Result<Corpus> {
    let custom = data.join("bench.txt");
    let path = if custom.exists() {
        custom
    } else {
        install
            .join("bench")
            .join(format!("{}_lines.txt", lang.code()))
    };
    let text = std::fs::read_to_string(&path)?;
    let lines = text
        .lines()
        .map(|s| s.trim_start_matches('\u{feff}').trim())
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .map(str::to_string)
        .collect();
    Ok(Corpus { path, lines })
}

pub struct Latency {
    pub mean: u128,
    pub median: u128,
    pub p90: u128,
}

/// Successful requests only; a genuine zero-millisecond result is retained.
pub fn latency(times: &[u128]) -> Option<Latency> {
    if times.is_empty() {
        return None;
    }
    let mut sorted = times.to_vec();
    sorted.sort_unstable();
    let n = sorted.len();
    let median = if n % 2 == 0 {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2
    } else {
        sorted[n / 2]
    };
    Some(Latency {
        mean: sorted.iter().sum::<u128>() / n as u128,
        median,
        p90: sorted[(9 * n).div_ceil(10) - 1],
    })
}
