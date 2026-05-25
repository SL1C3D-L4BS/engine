# ADR-006 — WGSL and WebTransport for the web target

- Status: Accepted
- Date: 2026-05-18 (expanded 2026-05-24 per audit §15 Phase-0 ADR sweep)
- Phase: 0 (pre-v1.0 — contract-exempt per risk R-03)
- Companion: ADR-003 (Slang authoring — WGSL is the web output
  target), ADR-037 (slangc toolchain — emits WGSL via
  `--target wgsl`), ADR-009 (two netcode modes — informs the
  WebTransport choice on the netcode side), Phase 9 (future
  netcode realisation)

## Context

The engine has two production target families:

- **Native** (Linux, Windows, macOS — eventually mobile/console).
  Vulkan for graphics; UDP for game-server transport. Full
  threading, full filesystem, full GPU access.
- **Web** (browser via WebAssembly + WebGPU). Restricted: no
  threads outside SharedArrayBuffer/Web Worker setup; no raw
  UDP; no direct file access (beyond the File System Access
  API on certain browsers). GPU access via WebGPU only.

The web target is a first-class deliverable, not a stretch goal:
the spec's distribution model (spec §III) explicitly includes
"play in browser" for game shipping. Two long-standing
architectural choices have to be made up-front so the rest of
the engine can target both:

1. **Shader language emission for the web.** WebGPU's only
   accepted shader language is WGSL; the engine must emit WGSL.
2. **Network transport on the web.** Raw UDP is unavailable;
   WebTransport over HTTP/3 (QUIC) is the closest equivalent,
   newly supported in production browsers and matched by a
   server-side QUIC stack.

## Decision

- **Shaders** are authored in Slang (ADR-003) and emitted to
  WGSL via `slangc --target wgsl` at asset-build time (ADR-037).
  The shader pak (ADR-008) carries the WGSL bytes; the web
  runtime loads WGSL from the pak and hands it to WebGPU. No
  shader compilation at runtime in the browser.
- **Network transport** uses UDP on native (via the
  `engine-net` crate's owned QUIC implementation) and
  WebTransport-over-QUIC on the web. The transport abstraction
  is unified: both paths look like "send a datagram or a
  reliable stream" to the netcode layer. Phase 9 realises this
  contract; Phase 0 records the choice so no Phase-9 surprise
  forces an engine-wide retrofit.

The engine emits the same compiled-shader bytes everywhere
identical-target SPIR-V on Vulkan and WGSL bytes on WebGPU
yield the same observable behaviour, modulo legal differences
(WGSL doesn't expose some Vulkan extensions; the engine's
Phase-5 shader surface does not depend on any of them).

## Rationale

WGSL is the only shader language WebGPU accepts. The alternative
to "emit WGSL" is "don't ship a web target," which the spec
explicitly rejects. Slang's WGSL emission (Khronos-owned
toolchain) gives the engine a single source-of-truth shader
language; the cost of WGSL is paid by the asset-build pipeline,
not by shader authors.

WebTransport is the only QUIC-style datagram-and-stream API
generally available in browsers in 2026. The alternative
(WebRTC data channels) carries SDP/ICE complexity, NAT
traversal cost, and per-peer overhead unsuitable for a
client/server topology. WebSockets are TCP-only — no datagram
semantics, head-of-line blocking on packet loss.

The native UDP / web WebTransport split keeps the netcode
trade clean: both modes are connection-oriented (post-handshake)
and both support unreliable datagram and reliable stream sends.
The netcode layer (Phase 9) writes once against the unified
trait; the per-target adapter is small.

## Consequences

- The shader pak format (ADR-008) carries WGSL bytes for every
  shader, in addition to SPIR-V / MSL / HLSL bytes per the
  spec's other target backends. Pak size grows roughly 1.4× per
  shader; the content-addressing makes the cost a one-time per
  shader/per slangc-version compile.
- The engine's network layer (Phase 9) ships two transport
  backends: `engine-net::native::udp` (Linux/Windows/Mac/mobile)
  and `engine-net::web::webtransport` (wasm32 target). The
  shared trait surface lives in `engine-net::Transport`.
- The engine's wasm32 build excludes the native backends
  cleanly via `cfg(target_arch = "wasm32")`. No conditional
  compilation in user code.
- WebGPU's lack of compute-shader features available in Vulkan
  (subgroup ops on some implementations, ray tracing) means
  Track B (ADR-004) work-graph rendering is native-only for
  the foreseeable future. The web target stays on Track A.

## Risks and tradeoffs

- **WebGPU spec churn.** The WGSL spec evolves; browser
  implementations sometimes lag. Mitigation: the slangc version
  pin (ADR-037) and the shader-output reproducibility golden
  (ADR-038) make a WGSL spec change visible at build time, not
  at user-runtime.
- **WebTransport browser support gaps.** Safari's WebTransport
  rollout has been slower than Chromium / Firefox. Mitigation:
  Phase 9 will need a fallback transport (WebSocket-over-TCP,
  acknowledged as lower-quality) or a "Safari unsupported"
  posture for online play. Decision deferred to Phase 9.
- **No threads on the web** (without specific COOP/COEP headers
  + SharedArrayBuffer). The ECS scheduler's parallel dispatch
  (ADR-033) compiles on wasm32 but runs single-threaded by
  default. Acceptable: the web target's portfolio is smaller
  scenes; the milestone bench (ADR-047) is native-only.
- **WGSL feature parity** with Vulkan-targeting Slang output.
  Some Slang patterns (autodiff, generic specialisation past a
  certain depth) may not survive the WGSL emission. The Slang
  WGSL backend's matrix is evolving; the test corpus will
  expose gaps.

## Alternatives considered

- **Emit SPIR-V to the browser** (via SPIR-V Cross or similar
  at runtime). Browsers do not accept SPIR-V on WebGPU.
  Rejected (not technically possible).
- **A separate browser-only renderer.** Doubles maintenance;
  the same renderer/shaders should produce the same image.
  Rejected.
- **WebSocket transport instead of WebTransport.** TCP head-of-
  line blocking is a poor fit for realtime netcode. Rejected.
- **WebRTC data channels.** Higher integration cost (SDP/ICE);
  per-peer overhead poor for client/server. Rejected.
- **Skip the web target.** The spec rejects this. Not
  considered seriously.

## Verification

- The slangc toolchain (ADR-037) emits valid WGSL for the entire
  Phase-5 shader corpus; the WGSL passes Naga's WebGPU
  validator in CI.
- The shader-output reproducibility golden (ADR-038) covers WGSL
  as a target; any WGSL emission change is a visible diff.
- Phase 9 verification: the unified `engine-net::Transport`
  trait passes the netcode test suite under both native UDP
  and web WebTransport adapters.
- The web target's wasm32 build compiles in CI (no run-time
  test on wasm32 in CI today; that gates after the WebGPU
  headless test runner lands in Phase 9+).
