# ADR-019 — Asset sandbox subprocesses

- Status: Accepted
- Date: 2026-05-18 (expanded 2026-05-24 per audit §15 Phase-0 ADR sweep)
- Phase: 0 (pre-v1.0 — contract-exempt per risk R-03)
- Companion: ADR-018 (plugin sandboxing — broader case), ADR-037
  (slangc toolchain — concrete realisation of this pattern),
  ADR-045 (texture compression — Phase 5 PR 2 also runs an
  asset-sandbox subprocess)

## Context

Asset importers operate on untrusted input files. The 2026
threat model includes:

- An FBX file crafted to overflow a parser buffer.
- A PNG with adversarial chunk metadata.
- A glTF model whose embedded shader code does something
  malicious.
- A texture file that triggers a known CVE in `libpng` /
  `libjpeg-turbo` / `libheif`.

A direct in-process FBX importer in the editor is one zero-day
away from arbitrary code execution. The engine's editor runs
on developer machines; an arbitrary-code-execution vulnerability
at editor scale is a supply-chain incident.

The cleanest mitigation is the same one Chrome's image decoder,
ffmpeg's frontend, and modern PDF renderers settled on:
**parse-untrusted-input in a syscall-restricted subprocess.**
The editor never touches the bytes directly; the subprocess
parses, validates, and emits a normalised intermediate form;
the editor consumes the intermediate form.

## Decision

Asset importers operate as syscall-restricted subprocesses:

- Each importer is a separate binary
  (`engine-importer-fbx`, `engine-importer-obj`,
  `engine-importer-png`, etc.). FBX/OBJ importers are
  Phase 11+ (mobile/console roadmap); shader compilation
  (slangc per ADR-037) and texture compression (Phase 5 PR 2
  per ADR-045) are concrete present-day instances of the same
  pattern.
- The importer reads from stdin (or a memory-mapped temp file
  for large inputs); writes the normalised output to stdout
  (or another memory-mapped temp file).
- The importer runs under seccomp-bpf (Linux) /
  AppContainer/job-object (Windows) / sandbox-exec (macOS)
  with a syscall allowlist that excludes network, full
  filesystem, ptrace, fork, exec.
- **No network access.** Even for importers that nominally
  could need it (e.g. a remote-asset importer in some future
  ADR), the network call is the editor's job; the importer
  parses only.
- Crashes in the subprocess are caught by the editor's
  subprocess wrapper and reported as a typed
  `ImporterError::ParserCrash`; the editor survives.
- The subprocess wrapper applies a wall-clock timeout (default
  30 s) so adversarial inputs that send the parser into a
  malicious infinite loop are bounded.

The slangc subprocess (ADR-037) is the canonical implementation
pattern: a third-party binary (`slangc`), invoked via the
engine's subprocess wrapper, sandboxed with the same
seccomp/jobobject filter, with the same crash-survival
contract.

## Rationale

Three properties motivate the subprocess model:

1. **Crash isolation.** A malformed FBX that crashes the FBX
   parser does not crash the editor. The editor recovers and
   reports the bad asset.
2. **Privilege reduction.** A malicious FBX that achieves code
   execution inside the parser is contained: no network,
   no filesystem outside the sandbox, no engine-process
   memory access. The blast radius is the subprocess.
3. **Library-CVE absorption.** Importers often link against
   third-party native libraries (`libfbx`, `libpng`,
   `libheif`). CVEs in these libraries become subprocess-only
   exposures; the editor's main process is unaffected.

The pattern's industry precedent (Chrome, ffmpeg, Adobe's
post-2020 PDF renderer rewrite) is the validation: every
high-value editor that historically suffered import-parser
CVEs has converged on the subprocess model.

## Consequences

- The editor's importer surface is a CLI process pool — one
  warm subprocess per importer type, reused across imports.
- The intermediate form (importer output → engine asset) is
  the engine's own normalised format; importer authors target
  it. For Phase 5 PR 2's texture-compression case, the
  intermediate form is the `TextureMeta` + the compressed
  texture bytes (ADR-045).
- Subprocess startup cost is amortised by pool reuse.
- The seccomp-bpf filter for asset subprocesses is a shared
  Linux baseline (defined in `engine-platform::sandbox`).
- A small `engine-platform::subprocess` API wraps the
  spawn-sandboxed-process pattern; both the slangc invocation
  (`tools/engine-shader/src/slangc.rs`) and the texture
  compressor (Phase 5 PR 2) use this API.

## Risks and tradeoffs

- **Subprocess startup latency.** Spawn + sandbox setup is
  ~10 ms on Linux, ~30 ms on Windows. Mitigated by pool
  reuse for bulk imports.
- **IPC overhead.** Large assets (a 4K texture, a 100MB FBX)
  cross process boundaries. Mitigation: memory-mapped temp
  files for the input/output bytes; only metadata travels
  through stdin/stdout.
- **Per-platform sandbox differences.** Linux/Windows/macOS
  filters diverge; same as ADR-018. Mitigation: the
  `engine-platform::sandbox` API hides the per-platform
  implementation.
- **Adversarial CPU exhaustion.** A malicious asset designed
  to hang the parser inside its 30 s timeout window costs the
  editor 30 s. Mitigation: parallel imports proceed; the
  timeout fires and the asset is rejected.
- **Debugging across the subprocess boundary** is harder. A
  parser bug requires reproducing in a stand-alone harness.
  Mitigation: the importer's CLI is independently invocable
  with `--debug` to bypass the editor wrapper.

## Alternatives considered

- **In-process importers.** Simpler; loses the security
  property. Rejected for any importer parsing untrusted input.
- **WASM-sandboxed importers.** Solid security; loses access
  to native importer libraries (libfbx, libpng C deps);
  performance worse than native. Phase 11+ candidate.
- **VM-isolated importers** (e.g. each importer in a
  microVM). Excessive; subprocess + seccomp is sufficient.
  Rejected.
- **Trust-everything model.** What most editors did before
  the early-2010s wave of import-parser CVEs. The audit posture
  has shifted; not viable in 2026.

## Verification

- The slangc subprocess (ADR-037) is the in-production
  implementation; ADR-038's reproducibility golden is its
  oracle.
- The texture-compression subprocess (Phase 5 PR 2 per
  ADR-045) follows the same pattern; its oracle is the
  texture-decompression SSIM match.
- Phase 11+ FBX/OBJ importers will inherit the pattern; the
  audit will verify they did.
- The `engine-platform::subprocess` API has unit tests
  covering: timeout enforcement, sandbox filter setup,
  stdout/stderr capture, abnormal-exit detection.
- A red-team test: a deliberately malformed input is fed
  through the pipeline; the editor reports a typed error
  rather than crashing.
