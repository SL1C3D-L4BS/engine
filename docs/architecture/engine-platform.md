# engine-platform

Operating-system abstraction (spec IV.1 Level 0, IV.5).

## Purpose

Isolates the rest of the engine from the host OS: time, the filesystem,
filesystem change watching, host information, and input event vocabulary.
This is the *foundation slice* — windowing and GPU surface creation depend on
a compositor and arrive with the renderer in a later phase.

## Modules

| Module     | Contents |
|------------|----------|
| `time`     | Monotonic clock and `FramePacer` — coarse sleep plus a short busy-wait spin to hit the frame boundary precisely (spec IV.5, ADR-016). |
| `fs`       | Path helpers and `atomic_write` — write-temp-then-rename, so a crash never leaves a half-written file. |
| `watch`    | `FileWatcher` trait with a portable `PollingWatcher` and a Linux `InotifyWatcher`; the basis of asset and script hot-reload. |
| `sysinfo`  | Build target architecture/OS, core count, page size. |
| `input`    | Input event type definitions (`Key`, `MouseButton`, `InputEvent`, …) — plain data, no device polling yet. |

## Design notes

- Frame pacing optimizes *consistency*, not peak rate: a long coarse sleep
  followed by a sub-millisecond spin minimizes both jitter and wasted CPU.
- `atomic_write` is the mandated path for any durable write by the asset
  pipeline or editor.
- The watcher API is non-blocking: `poll` returns changes observed since the
  previous call, so it composes with the engine's frame loop.

## Oracle

Unit tests cover sleep accuracy within tolerance, atomic-write crash-safety
(the temp file never appears as the final file), and an inotify round-trip
(create/modify/delete are observed).

## Dependencies

`libc` (raw syscalls) — Level 0.
