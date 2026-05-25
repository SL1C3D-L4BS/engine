# ADR-009 — Two netcode modes

- Status: Accepted
- Date: 2026-05-18 (expanded 2026-05-24 per audit §15 Phase-0 ADR sweep)
- Phase: 0 (pre-v1.0 — contract-exempt per risk R-03)
- Companion: ADR-013 (determinism contract — required for rollback
  mode), ADR-006 (WGSL + WebTransport — informs the web netcode
  transport), Phase 9 (future realisation)

## Context

A 50-year game engine must serve more than one network genre. The
two dominant patterns are technically incompatible — they cannot
share a single set of primitives without compromising both:

- **Client prediction with server reconciliation** ("authoritative"
  in the canonical netcode literature, Valve's Source pattern).
  Server is authoritative; clients predict their own actions and
  reconcile when the server disagrees. Tolerates non-deterministic
  simulation across machines because the server's truth is the
  one that survives reconciliation. Genre fit: shooters, MMOs,
  most action games.
- **Deterministic lockstep with rollback** ("rollback" in
  fighting-game / RTS netcode literature, GGPO/QUARK lineage).
  Every machine simulates every frame from the same inputs;
  divergence is impossible. Tolerates *no* simulation non-
  determinism. Genre fit: fighting games, RTSes, peer-to-peer
  competitive games with tight input timing.

The spec's stance (§IV.9, finding F-06): both modes must be
first-class; an engine that ships only one limits the portfolio.

## Decision

The `engine-net` crate exposes two distinct top-level netcode
modes, selected at session start and immutable thereafter:

- `Mode::Authoritative` — client-server topology, predict +
  reconcile, server-authoritative state.
- `Mode::Rollback` — peer-to-peer (or relayed) lockstep, full
  deterministic replay, GGPO-style rollback on input delay.

The engine **refuses to mix them within a session.** A game cannot
have some entities in authoritative mode and others in rollback;
the mode is a session-level invariant. Mode changes require a
new session.

The shared substrate underneath both modes:

- `engine_net::Transport` — the connection-oriented datagram +
  reliable-stream surface (UDP native, WebTransport on the web
  per ADR-006).
- `engine_net::Tick` — a tick counter shared by both modes; the
  determinism contract (ADR-013) makes a given tick reproducible
  in rollback mode and meaningful in authoritative mode.
- `engine_net::Snapshot` — the per-tick world state digest format,
  used for reconciliation (authoritative) and verification
  (rollback).

## Rationale

Two reasons the engine cannot collapse the two modes into one:

1. **Determinism cost.** Rollback requires the entire simulation
   to be deterministic — no `HashMap` iteration order, no float
   rounding differences, no scheduler jitter (ADR-033). The cost
   is paid up front by every system. Authoritative mode does not
   require this; reconciliation absorbs simulation drift.
   Imposing rollback's determinism cost on shooter genres would
   slow them down for no benefit. Imposing authoritative's
   reconciliation machinery on a fighting game would add latency.
2. **Topology and trust.** Authoritative mode has a server (the
   trust root); rollback mode has peers (no trust root). The
   anti-cheat surface, the connection model, the matchmaking
   integration — all differ.

The shared substrate (transport, tick, snapshot) carries enough
to make both modes feel like the same crate without forcing
either to compromise.

## Consequences

- Phase 9 ships both modes. `engine_net` is a substantial crate.
- The determinism contract (ADR-013) is mandatory for the entire
  engine, not just for rollback users. This is acceptable:
  every system is already on the determinism gate (the netcode
  benefit is one of many).
- The session-level mode choice is part of the game's
  `engine.toml` (Phase 9 will define the schema). Mode mismatch
  on connection (a server expecting authoritative, a client
  expecting rollback) is a typed connection-refused error.
- Each mode has its own anti-cheat surface; the engine ships
  hooks for plugin-based anti-cheat per ADR-018 (plugin
  sandboxing).

## Risks and tradeoffs

- **Implementation cost.** Two netcode modes is genuinely two
  netcode subsystems. Phase 9 is sized accordingly.
- **Determinism slip** in authoritative-only games could
  silently make them non-rollback-capable. Mitigation: the
  determinism oracle (ADR-013) runs against every engine
  build; non-determinism is a CI failure regardless of which
  mode the game uses.
- **WebTransport latency variance** in browsers could degrade
  rollback's input-prediction window. The web target may need
  per-platform tuning of rollback's frame delay envelope.
- **Anti-cheat in rollback (no server-of-truth)** is a hard
  problem. Mitigation: shared anti-cheat plugin API per
  ADR-018; the engine doesn't pick a vendor up front.

## Alternatives considered

- **Authoritative only.** Covers the majority of network genres;
  fails fighting games and RTSes. Rejected against the
  portfolio constraint.
- **Rollback only.** Determinism cost is universal; fails to
  give shooters their reconciliation tools. Rejected against
  the portfolio constraint.
- **A unified "hybrid" mode** that picks per-entity. Sounds
  appealing; in practice means "do both badly." Every
  engine that tried this in the literature backed out.
  Rejected.
- **No netcode in the engine; consumers bring their own.**
  Loses the integration with the determinism contract (ADR-013)
  and the snapshot/replay infrastructure; loses the ECS-aware
  delta-encoding optimisation. Rejected.

## Verification

- Phase 9 will land a netcode parity oracle: a small workload
  runs in both modes and produces matching deterministic
  digests on the rollback side and matching reconciled state
  on the authoritative side.
- The transport layer's unit tests exercise UDP and
  WebTransport adapters against the shared trait.
- Mode-mismatch connection refusal has a unit test.
- Snapshot format reproducibility is part of the determinism
  CI gate.
- The deferred fix for the spec's finding F-06 is *resolved*
  by the formal ADR-009 expansion + Phase-9 implementation.
