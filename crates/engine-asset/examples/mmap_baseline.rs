//! mmap'd-pak baseline (ADR-029).
//!
//! Builds a synthetic pak (~256 MiB, 10 000 blobs) on a scratch path and
//! reports:
//!
//! - Load-time wall-clock for `Pak::from_bytes(fs::read(path))` vs.
//!   `Pak::open_mmap(path)`.
//! - Resident-set-size delta (`VmRSS` from `/proc/self/statm`) after each
//!   load.
//! - Cold-cache first-touch latency on a small random sample of blobs
//!   versus a warm-cache repeat.
//!
//! Run via `just mmap-baseline`. The output is markdown; redirect to
//! `docs/observatory/mmap-asset-baseline.md` to refresh the committed
//! baseline.

use engine_asset::Pak;
use std::hint::black_box;
use std::time::Instant;

const TARGET_BYTES: usize = 256 * 1024 * 1024;
const BLOB_COUNT: usize = 10_000;
const SAMPLE_READS: usize = 10;

fn main() {
    let blob_size = TARGET_BYTES / BLOB_COUNT;

    // Build the pak in memory. We don't time this — the baseline measures
    // *load*, not *create*. ~256 MiB of allocation here is intentional;
    // run on a host with the headroom.
    let mut builder = Pak::builder();
    let mut state: u64 = 0xDEAD_BEEF_C0DE_C0DE;
    for i in 0..BLOB_COUNT {
        let mut blob = Vec::with_capacity(blob_size);
        for _ in 0..(blob_size / 8) {
            state = state.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1);
            blob.extend_from_slice(&state.to_le_bytes());
        }
        builder.add(format!("blob/{:05}.bin", i), blob);
    }
    let pak = builder.build();
    let bytes = pak.to_bytes();
    let pak_path =
        std::env::temp_dir().join(format!("engine-mmap-baseline-{}.pak", std::process::id()));
    std::fs::write(&pak_path, &bytes).expect("write synthetic pak");
    drop(pak); // free the in-memory pak

    let rss_at_start = read_rss_kib();

    // --- Path A: from_bytes(fs::read(path)) ---
    let t0 = Instant::now();
    let read_bytes = std::fs::read(&pak_path).expect("read pak");
    let pak_a = Pak::from_bytes(&read_bytes).expect("decode pak");
    let load_a = t0.elapsed();
    let rss_after_a = read_rss_kib();
    drop(read_bytes);

    // Pick a deterministic sample of blob names to measure cold/warm
    // first-touch latency. We need to do this before dropping pak_a so we
    // can compute the same names for both paths.
    let sample_names: Vec<String> = sample_blob_names(BLOB_COUNT, SAMPLE_READS);

    let mut sum: u64 = 0;
    let t_cold_a = Instant::now();
    for name in &sample_names {
        let bytes = pak_a.get(name).expect("blob present");
        sum = sum.wrapping_add(bytes[0] as u64);
    }
    let cold_a = t_cold_a.elapsed();
    let t_warm_a = Instant::now();
    for name in &sample_names {
        let bytes = pak_a.get(name).expect("blob present");
        sum = sum.wrapping_add(bytes[0] as u64);
    }
    let warm_a = t_warm_a.elapsed();
    black_box(sum);
    drop(pak_a);

    // --- Path B: open_mmap(path) ---
    let t0 = Instant::now();
    let pak_b = Pak::open_mmap(&pak_path).expect("open_mmap pak");
    let load_b = t0.elapsed();
    let rss_after_b = read_rss_kib();

    let mut sum_b: u64 = 0;
    let t_cold_b = Instant::now();
    for name in &sample_names {
        let bytes = pak_b.get(name).expect("blob present");
        sum_b = sum_b.wrapping_add(bytes[0] as u64);
    }
    let cold_b = t_cold_b.elapsed();
    let t_warm_b = Instant::now();
    for name in &sample_names {
        let bytes = pak_b.get(name).expect("blob present");
        sum_b = sum_b.wrapping_add(bytes[0] as u64);
    }
    let warm_b = t_warm_b.elapsed();
    black_box(sum_b);
    drop(pak_b);

    let _ = std::fs::remove_file(&pak_path);

    // --- Markdown report ---
    println!("# mmap'd-pak baseline");
    println!();
    println!("- **Synthetic pak**: {BLOB_COUNT} blobs × ~{blob_size} B ≈ 256 MiB.");
    println!(
        "- **Initial RSS**: {} KiB. After `from_bytes(fs::read)`: {} KiB \
         (Δ = {} KiB). After `open_mmap`: {} KiB (Δ = {} KiB).",
        rss_at_start,
        rss_after_a,
        rss_after_a.saturating_sub(rss_at_start),
        rss_after_b,
        rss_after_b.saturating_sub(rss_at_start),
    );
    println!();
    println!("| measurement | from_bytes(fs::read) | open_mmap |");
    println!("| --- | ---: | ---: |");
    println!("| load wall-clock | {:?} | {:?} |", load_a, load_b);
    println!(
        "| cold-cache {}-blob random read | {:?} | {:?} |",
        SAMPLE_READS, cold_a, cold_b
    );
    println!(
        "| warm-cache {}-blob random read | {:?} | {:?} |",
        SAMPLE_READS, warm_a, warm_b
    );
    println!();
    println!(
        "Methodology: synthetic blobs hashed with a multiplicative-congruential \
         PRNG so the bytes don't compress; the pak is written to `$TMPDIR` \
         (typically tmpfs) and the path is `unlink(2)`-ed after the report. \
         Cold-cache numbers include the first page-fault touch under mmap; \
         warm-cache numbers measure the same pointer chase after the kernel \
         has populated the relevant pages."
    );
}

fn sample_blob_names(n_blobs: usize, k: usize) -> Vec<String> {
    let mut state: u64 = 0x1234_5678_9ABC_DEF0;
    (0..k)
        .map(|_| {
            state = state.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1);
            let i = (state as usize) % n_blobs;
            format!("blob/{:05}.bin", i)
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn read_rss_kib() -> usize {
    // /proc/self/statm format: size resident shared text lib data dt — all
    // in pages. We want resident-set in KiB.
    let statm = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    let mut it = statm.split_whitespace();
    let _size = it.next();
    let resident_pages: usize = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let page_kib = 4; // overwhelmingly common; reading sysconf would be portable but heavier
    resident_pages * page_kib
}

#[cfg(not(target_os = "linux"))]
fn read_rss_kib() -> usize {
    0
}
