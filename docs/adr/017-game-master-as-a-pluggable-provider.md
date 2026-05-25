# ADR-017 — Game Master as a pluggable provider

- Status: Accepted
- Date: 2026-05-18 (expanded 2026-05-24 per audit §15 Phase-0 ADR sweep)
- Phase: 0 (pre-v1.0 — contract-exempt per risk R-03)
- Companion: ADR-018 (plugin sandboxing — the GM may be an
  out-of-process plugin), spec §IV.11, spec §III.4 (AI-driven
  content), Phase 8 (future realisation)

## Context

The engine supports AI-driven game content — procedurally
generated narrative, dynamic dialogue, NPC behaviour shaped by
a language model, encounter design driven by a "Game Master"
agent. The dominant technology in 2026 is local quantized LLMs
(7B–70B parameters, 4-bit quantised, running on consumer GPUs);
the secondary technology is hosted cloud LLMs (paid, more
capable, network-dependent).

The technology turnover is fast:

- 2024: Llama 2 7B, GPT-3.5-class capability.
- 2025: Llama 3, Mistral 7B v0.3, Claude 3.7 — substantial
  capability lift.
- 2026: Llama 4, Claude 4.7, Gemini 3 — substantial lift again.
- 2027+: unpredictable; the only certainty is "the model that's
  best in 2027 is not the model that's best in 2024."

A 50-year engine cannot hardcode a model identity. The
*integration surface* — how the game describes what it wants
from the GM, how the GM returns structured results, how the
GM's quality is evaluated against a benchmark — is the durable
layer. The model behind the integration is exchangeable.

## Decision

The engine exposes a `GameMasterProvider` trait. The contract:

```rust
pub trait GameMasterProvider {
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> &Capabilities;
    fn request(&mut self, req: GmRequest) -> GmResult<GmResponse>;
}
```

Where:

- `GmRequest` is structured (the game's "what I want" — e.g.
  a dialogue request with character + topic + constraints).
- `GmResponse` is structured (tool calls, structured outputs;
  the GM does not emit free-form text the engine then
  unstructured-parses).
- `Capabilities` declares the provider's supported tool surface
  (which tools, which output formats, which constraints).

Providers shipped with the engine (Phase 8):

- `Provider::LocalLlama` — local llama.cpp / mistral.rs binding.
- `Provider::CloudClaude` — hosted Anthropic API (opt-in;
  network-egress only with telemetry consent + per-request user
  consent).
- `Provider::CloudOpenAi` — hosted OpenAI API (same gating).
- `Provider::Mock` — a deterministic mock for testing/CI; the
  default in the determinism oracle's scope.

An **eval suite** ships alongside: a corpus of test scenarios
(dialogue requests, encounter requests, scene-description
requests) with known-good and known-bad reference outputs. New
providers are evaluated against the corpus; failing scenarios
are surfaced as a per-provider quality report. The eval suite
is the *durable* part — even when a model is replaced, the
suite tells whether the replacement is better, equivalent, or
worse on the engine's portfolio.

## Rationale

The decoupling is the entire point: the integration is the
durable layer, the model is the swappable layer. Three
properties make this work:

1. **Structured tool calls** (vs. unstructured-text parsing).
   2024-era prompt engineering relied on parsing free-form text
   for the model's response; 2026-era models support structured
   tool calls natively. The engine binds against the structured
   API; provider implementations expose it.
2. **Eval suite as the quality contract.** Without an eval
   suite, swapping models is a leap of faith; with the suite,
   the swap is a measurement.
3. **Determinism-friendly fallback.** The `Mock` provider is
   deterministic; the determinism oracle (ADR-013) uses it.
   Real model providers are not deterministic and are *not* on
   the determinism contract; games that need rollback netcode
   (ADR-009) must either not use the GM or use a deterministic
   subset of its surface.

## Consequences

- `engine-ai` is the crate that hosts the trait + the eval suite.
- Real model providers are likely *out-of-process plugins*
  (ADR-018), running in a sandboxed subprocess. The plugin
  pattern keeps the engine free of model-runtime dependencies
  (the llama.cpp/mistral.rs binding lives in the plugin, not
  the engine).
- The eval suite grows with every game's needs; Phase 8 ships
  a starter corpus.
- Cloud providers are network-egress and consent-gated
  (ADR-020); a session with cloud GM use must record consent
  before making the first request.
- The GM is not on the determinism path. Games that use the GM
  in rollback netcode must either replay the GM's outputs
  from a recorded session or use the `Mock` provider in
  rollback. Documented in ADR-009.

## Risks and tradeoffs

- **Model quality varies wildly.** A `Provider::LocalLlama` on a
  7B model and `Provider::CloudClaude` on Claude 4.7 produce
  qualitatively different content. Mitigation: the eval suite
  surfaces the gap; the game's portfolio chooses the
  acceptable quality floor.
- **API surface evolution.** Provider APIs change; the trait's
  shape needs to be expressive enough to absorb 2024-era and
  2027-era tool-call protocols. Mitigation: the trait is
  versioned; per-provider adapters absorb API drift.
- **Privacy.** Cloud providers send game content to a third
  party. Mitigation: consent-gated; the engine's privacy
  documentation calls this out; per-request opt-out is
  available.
- **Eval suite as a bottleneck.** A high-quality eval suite is
  expensive to maintain. Mitigation: Phase-8 starter corpus +
  community contributions + automated regression detection.

## Alternatives considered

- **Hardcode a model.** Optimises for 2024; obsoleted within
  18 months. Rejected.
- **No GM integration; games bring their own.** Loses the
  engine's eval-suite and trait-surface investment; every game
  reinvents the integration. Rejected.
- **A single cloud-only GM.** Network-required; privacy-
  concerning; latency-bound. Rejected.
- **A single local-only GM.** Misses the cloud capability
  upside. Rejected.

## Verification

- Phase 8 ships the trait, the four providers, and a starter
  eval suite (`engine-ai/tests/eval_corpus/`).
- The `Mock` provider's outputs are byte-deterministic;
  `engine-ai`'s determinism is verified by the engine-wide
  determinism gate.
- Real provider outputs are *not* deterministic; their tests
  use property checks (e.g. "the response is valid JSON
  matching the request's tool schema") rather than exact-match
  golden tests.
- The eval suite produces a per-provider quality report; the
  CI surface captures the report as a build artefact (not a
  gate — quality is informational, latency/correctness is a
  gate).
- Consent gating (ADR-020) verified by the network-egress
  unit test: a session without consent cannot reach a cloud
  provider's endpoint.
