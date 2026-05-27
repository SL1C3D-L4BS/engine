# Frame-pacing runner runbook

Operational runbook for the self-hosted GPU runner that hosts the
ADR-047 frame-pacing CI gate. The runner is named
`self-hosted-gpu-rx6700xt` in `.github/workflows/ci.yml`; this
document is the procedural surface a maintainer follows to bring it
up, keep it green, and recover from outages.

## Hardware envelope

| Component | Specification |
|-----------|---------------|
| GPU | **AMD Radeon RX 6700 XT** (RDNA 2, 12 GiB GDDR6). One tier above the RX-580 milestone target. |
| CPU | **AMD Ryzen 7 5700G** (8c/16t, frequencies locked via the BIOS or `cpupower frequency-set`). |
| Memory | **32 GiB DDR4-3200** (2 × 16 GiB, dual-channel). |
| Storage | NVMe (sccache cache lives here; size to ≥ 100 GiB). |
| Network | Wired Ethernet. Wi-Fi is not stable enough for sccache fetches under contention. |
| Chassis | Bare-metal, dedicated. No virtualisation; no shared workload. |

The runner is **production hardware** for ADR-047 purposes: it is
treated as a piece of the CI pipeline, not a developer workstation.
A maintainer does not log in and run a browser, a game, or a build
of an unrelated project while a gate job is in-flight.

## OS / kernel envelope

| Component | Specification |
|-----------|---------------|
| Distro | Arch Linux. |
| Kernel | `linux-cachyos-bore` (spec Part XVIII.4). Pinned to the package version active at runner provisioning; updates roll through `runner-maintenance` PRs (see §"Driver rolls"). |
| Scheduler | `sched-ext` enabled. The BORE scheduler is the reason σ ≤ 1.04 ms is achievable; `CONFIG_SCHED_BORE=y` is non-negotiable. |
| Init | systemd. The GitHub Actions runner is a systemd service. |
| GPU driver | Mesa RADV (Vulkan 1.3). Pin the version: `mesa=24.x.y-1`, etc. — record the active pin in this document each time it changes. |

## GitHub Actions runner setup

1. **Create the runner registration token** in the repo's *Settings →
   Actions → Runners → New self-hosted runner*. Use the **Linux x64**
   tab; the token is single-use and expires in 60 minutes.
2. **Install the runner agent.** On the runner host:
   ```bash
   mkdir -p /opt/gha-runner && cd /opt/gha-runner
   curl -O -L https://github.com/actions/runner/releases/download/<ver>/actions-runner-linux-x64-<ver>.tar.gz
   tar xzf actions-runner-linux-x64-<ver>.tar.gz
   ./config.sh --url https://github.com/<owner>/<repo> --token <TOKEN> \
       --name engine-rx6700xt-01 \
       --labels self-hosted-gpu-rx6700xt,linux,x64,rx6700xt \
       --work _work
   ```
3. **Install as a systemd service.**
   ```bash
   sudo ./svc.sh install
   sudo ./svc.sh start
   sudo systemctl status actions.runner.<owner>-<repo>.engine-rx6700xt-01
   ```
4. **Verify** the runner appears in *Settings → Actions → Runners*
   with the label set above and a green "Idle" state.

## Provisioning verification

After install, kick off a dispatch of the `frame_pacing` job from a
test branch:

```bash
gh workflow run ci.yml --ref test/frame-pacing-bringup
```

Then watch the run from `gh run watch`. The job must:

- Spawn on `engine-rx6700xt-01` (not a GitHub-hosted runner).
- Build `engine-bench-frame-pacing` and produce a JSON report at
  `/tmp/frame-pacing.json`.
- Print the gate verdict text. With `continue-on-error: true` set,
  the verdict can be FAIL on first-light without blocking; the job
  exit is still 0.

## "Going green" — the first required-status promotion

Once the runner has produced **3 consecutive green runs** on `main`
with consistent p99 / σ numbers (read from the uploaded
`frame-pacing-report` artifact), the gate is promoted:

1. Open a **`runner-maintenance` PR** that drops the
   `continue-on-error: true` line from `.github/workflows/ci.yml`'s
   `frame_pacing` job.
2. Add the job to the **branch-protection required-status list** for
   `main` (*Settings → Branches → main → Require status checks to
   pass before merging → frame_pacing*).
3. Record the promotion event in
   `docs/observatory/phase-5-milestone-baseline.md`'s History table
   under "Transition to PR 6 — required gate active".

Until the promotion lands, the gate's verdict is informational. The
runbook is the procedural commitment to the promotion path; the
runner is the dependency.

## sccache cleanup

The runner uses sccache for `cargo build` artefacts to keep gate
turnaround under 2 minutes. The cache grows over time and can
balloon past the NVMe budget.

- **Weekly** (cron): `sccache --stop-server && rm -rf
  ~/.cache/sccache/ && sccache --start-server`. Documented in the
  runner's `crontab -e`.
- **On low-disk** (`df -h /` < 10 GiB free): immediate cleanup.
- **After a driver roll** (see below): full cache flush. Driver
  changes can invalidate kernel-mode hashes that sccache miscaches.

## Driver rolls

Mesa / kernel updates roll through a **dedicated `runner-maintenance`
PR**, not the engine's main flow. The procedure:

1. Author opens a `runner-maintenance` PR titled e.g. "Mesa 24.3 →
   24.4 roll".
2. The PR body lists:
   - Old version, new version.
   - The package's changelog summary.
   - The expected effect on the gate (typically: "no change to
     thresholds expected; baseline numbers may shift within the
     noise band").
3. **Maintenance window.** A maintainer:
   - Pauses the runner (`sudo svc.sh stop`).
   - Performs the update (`sudo pacman -Syu mesa linux-cachyos-bore`,
     etc.).
   - Restarts the runner.
   - Triggers a baseline run of the gate against `main`.
4. The PR records the baseline numbers (mean / p99 / σ) and is
   merged. If the baseline shows a regression > 5 %, the PR is held
   and an investigation issue opened — driver-attributable
   regressions are tracked separately from engine-attributable ones.

## Cold-spare procedure

ADR-047 §Risks requires a cold-spare on a different physical host
in case the primary runner fails. Capacity planning: the spare is
provisioned identically to the primary; it stays powered off until
needed.

When the primary fails:

1. Power on the cold-spare.
2. Rotate the GitHub Actions registration token (the primary's may
   still be live; the spare needs its own runner identity).
3. Run the registration steps from §"GitHub Actions runner setup",
   naming the runner `engine-rx6700xt-02` and using the same labels.
4. The primary's runner identity stays registered but Idle until the
   primary returns; CI scheduling picks whichever is available. The
   `needs: gate` keeps both runners from racing on the same PR.
5. Open a `runner-maintenance` PR titled "RX 6700 XT cold-spare
   activation YYYY-MM-DD" capturing the swap event, the suspected
   failure mode of the primary, and the first three baseline runs
   on the spare.

When the primary returns: the spare can either stay in service (with
the primary becoming the new cold-spare) or be powered off. The
choice is documented in the next `runner-maintenance` PR.

## Outage handling

A frame-pacing outage is **not** a release blocker so long as the
gate is in informational mode (PR-5 / PR-6 pre-promotion). Once
promoted to required status:

- **< 4 hours.** Wait. The runner may recover on its own (transient
  network blip, sccache crash, etc.). The maintenance person on call
  watches the runner's systemd status.
- **4–24 hours.** Activate the cold-spare (see above).
- **> 24 hours.** Open an incident issue. Consider temporarily
  removing the gate from required status while the investigation
  proceeds; this is a high-friction action and is documented in the
  issue.

## Telemetry / observability

- The runner emits its system metrics to a private Prometheus
  endpoint (consent-gated per ADR-020). Tracked metrics: CPU
  frequency lock, GPU power state, sccache hit rate, sched-ext
  occupancy.
- The bench JSON reports are uploaded as workflow artifacts
  (`frame-pacing-report`, 30-day retention). They are the
  authoritative input for the observatory baseline file.

## See also

- `docs/adr/047-frame-pacing-ci-gate.md` — the gate contract.
- `docs/adr/016-frame-pacing-contract.md` — the metric definitions.
- `docs/observatory/phase-5-milestone-baseline.md` — the baseline
  file the gate compares each run against.
- `tools/frame-pacing/budgets.toml` — the budget thresholds the
  `--gate` flag reads.
