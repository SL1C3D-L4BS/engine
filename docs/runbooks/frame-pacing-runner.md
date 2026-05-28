# Frame-pacing local runbook (ADR-070)

Operational runbook for the local frame-pacing bench. Replaces the
prior "self-hosted CI runner" runbook per ADR-070 (Phase 5.5 C.2 —
2026-05-27): the GitHub-Actions `frame_pacing` job was removed; the
bench survives as a local `just frame-pacing` recipe that the
developer runs before every PR to main.

Prior content (CI runner provisioning, OS pinning, sccache layout,
sched-ext requirements) is preserved in git history at the ADR-070
commit. If/when a second contributor joins, restore the workflow
stanza and consult the prior runbook for the operational details.

## Hardware envelope (the developer's machine)

| Component | Specification |
|-----------|---------------|
| GPU | **AMD Radeon RX 580** (Polaris 10, GFX8, 8 GiB GDDR5). The spec's named Recommended-tier hardware (Part XX.7 line 1587) and Phase 5 milestone target (line 1631). |
| CPU | **Intel Core i7-6700** (Skylake, 4c/8t @ 3.4–4.0 GHz, AVX2 + FMA, no AVX-512, 8 MiB L3). |
| Memory | **31 GiB DDR4**. |
| Storage | NVMe + SATA SSD mix; sccache lives in `~/.cache/sccache/`. |
| Network | Wired Ethernet preferred for any cargo registry fetches. |

The hardware is **not** a dedicated CI machine — it's the developer's
workstation. Background load minimisation (below) is the operational
guard against jitter contamination.

## Kernel envelope

| Component | Specification |
|-----------|---------------|
| Distro | Arch / CachyOS hybrid. |
| Kernel | `linux-cachyos-bore` (spec Part XVIII.4). Confirm via `uname -r` — substring `cachyos-bore` should appear. |
| Scheduler | BORE (Burst-Oriented Response Enhancer). The σ envelope per McKenney's analysis (ADR-070 §References) assumes BORE; alternative schedulers may exceed the σ budget. |
| GPU driver | Mesa RADV (Vulkan 1.3 on Polaris). Confirm via `vulkaninfo --summary` (look for "AMD Radeon RX 580 Series" + "Vulkan 1.3"). |

## Pre-bench checklist

Before running `just frame-pacing` for an authoritative number:

1. **Close non-essential applications.** Browser, IDE language servers,
   any media player. The bench is sensitive to scheduler contention.
2. **Check CPU thermals are not throttling.** Run `watch -n 1 'cat /sys/class/thermal/thermal_zone*/temp'`
   for 30 s; sustained > 90 °C means the CPU may dynamically downclock
   and inflate σ.
3. **Confirm no background `cargo` / `cc1` processes.** `pgrep cargo`
   should return only the bench's own PID once it's running.
4. **Plug in (laptop only).** Battery operation triggers conservative
   CPU governor which inflates p99 and σ. Not relevant to the
   developer's desktop i7-6700 but documented for completeness.

## Standard scenario

Per ADR-047 §1, the standard scenario is
`testbed/frame-pacing/scenes/v0.ron` (3600 frames, 10 000 entities,
60-second deterministic camera path). The scene is content-addressed
(BLAKE3) so a change to the fixture is reviewable.

## Running the bench

The canonical invocation:

```fish
just frame-pacing
```

This:
1. Builds the release binary (cached after first run).
2. Runs the bench on the v0 scene; writes
   `target/frame-pacing-latest.json`.
3. Evaluates `--gate` against `tools/frame-pacing/budgets.toml`.
4. Prints `PASS` or `FAIL` with the measured p99 / σ vs the budgets.

A single run takes ~5 minutes today (the bench's CPU-rasterizer
oracle workload at 1280×720 internal is intentionally slow; A.2b-ii's
GPU-path swap will bring per-frame cost to within the 16.6 ms
budget).

## Repeat-run protocol

For authoritative numbers (e.g. before an ADR-070 amendment that
re-baselines the budget):

1. Run the bench 10 consecutive times. Discard the first as warm-up.
2. Compute the median across runs (not the mean — a single outlier
   run can shift the mean disproportionately).
3. Take the median + 5 % headroom as the candidate new budget.
4. File the ADR-070 amendment with:
   - Per-run p99 / σ / mean / min / max.
   - Median, headroom calculation, candidate new budget.
   - The measurement protocol used (background load, kernel version,
     Mesa version, GPU driver version).
5. Update `tools/frame-pacing/budgets.toml`.
6. Update `docs/observatory/phase-5-milestone-baseline.md` with the
   measurement appendix.

## USE-method debugging (Gregg, *Systems Performance* App. A)

If a bench run produces unexpectedly bad numbers, walk the USE
method per resource:

| Resource | Utilization | Saturation | Errors |
|---|---|---|---|
| CPU | `mpstat -P ALL 1` (per-core %idle) | `vmstat 1` (runqueue length `r`) | `perf stat -a sleep 5` |
| Memory | `free -h` (% used) | `vmstat 1` (`si` / `so` non-zero = swapping) | `dmesg \| grep -i oom` |
| GPU | `radeontop` (gpu %) | gpu vram %, gtt % | `dmesg \| grep -i amdgpu` |
| Disk | `iostat -xz 1` (%util) | `iostat` (`avgqu-sz`) | `smartctl -a /dev/nvme0n1` |

A run that fails the budget AND shows saturation on a non-bench
resource is contamination; re-run with the contaminating process
killed before quoting numbers.

## When to amend ADR-070

- Re-baselining the budget (per ADR-047 §5's appeal procedure, now
  inherited by ADR-070).
- Hardware upgrade (CPU / GPU / kernel change). Re-measure first;
  amend with the new envelope description.
- Switching from CPU-oracle to GPU-path workload (A.2b-ii's
  milestone). The CPU baseline becomes a historical anchor; the GPU
  numbers become the live contract.

## See also

- `docs/adr/070-frame-pacing-rebaseline-rx580.md` — this runbook's
  governing ADR.
- `docs/adr/047-frame-pacing-ci-gate.md` — the prior CI-runner gate
  (now superseded for hardware / runner / threshold sections).
- `docs/adr/016-frame-pacing-contract.md` — the metric definitions.
- `docs/observatory/phase-5-milestone-baseline.md` — measurement
  appendix.
- `tools/frame-pacing/budgets.toml` — the budget thresholds.
