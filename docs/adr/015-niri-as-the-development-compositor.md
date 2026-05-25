# ADR-015 — Niri as the development compositor

- Status: Accepted
- Date: 2026-05-18 (expanded 2026-05-24 per audit §15 Phase-0 ADR sweep)
- Phase: 0 (pre-v1.0 — contract-exempt per risk R-03)
- Companion: spec finding F-02 (Niri), spec §XVIII.6 (developer
  environment)

## Context

The engine's developer environment is opinionated. The spec's
§XVIII names a baseline Linux setup (Arch + a specific kernel
+ a specific compositor) so the team's development experience
is reproducible. The relevant choice here is the Wayland
compositor.

The 2026 Wayland compositor landscape:

- **Sway** (and i3 forks). Stacking tile + floating; the
  workhorse of the i3-derived users for ~5 years. Solid; no
  fresh investment; C codebase.
- **Hyprland.** Eye-candy compositor; many features; C++
  codebase; significant churn; not the project's idiom.
- **GNOME / KDE.** Full desktop environments; not the
  project's idiom either.
- **Niri** (yalter/niri). Scrollable-tiling. Rust-native.
  Daily-drivable in 2026. Live-config-reload. NVIDIA support
  via the Wayland NVIDIA EGL path. Actively developed,
  yalter's roadmap consistent.

The spec finding F-02 names Niri as the chosen compositor for
the team's development environment. This ADR records the
decision.

## Decision

The team's development environment runs **Niri** as the Wayland
compositor. The configuration lives in `dotfiles/niri/` (not in
this repository — outside the engine's source tree).

Properties relied upon:

- Scrollable-tiling layout matches the multi-tool workflow
  (editor + terminal + browser + reference docs + engine TUI +
  scratchpad). Workspace overflow is the norm; horizontal
  scroll handles it naturally.
- Live-config-reload: changes to `~/.config/niri/config.kdl`
  apply without a session restart.
- NVIDIA-supported: the team's reference workstation includes
  NVIDIA hardware; Niri's NVIDIA EGL path is stable in 2026.
- Daily-drivable: not a research toy; the maintainer's own
  workflow.
- Rust-native: aligns with the engine's language stance
  (ADR-001).

## Rationale

The compositor choice is a per-developer environment decision;
its presence in the engine's ADR set is documentation, not
enforcement. The reason it lives here at all: the engine's
visual tests (the planned Phase 10 editor screenshots, the
post-Phase-5 oracle exception register's photographic
references) are taken under a specific compositor; the
compositor's anti-aliasing / scaling / colour-management
choices affect the captured pixels.

Documenting the chosen compositor makes the reference
environment reproducible. A contributor on a different
compositor will produce slightly different screenshots; the
register's exception note will reflect that. Niri is the
canonical reference.

Beyond reproducibility, Niri's scrollable-tiling matches the
team's workflow. The multi-window-per-task pattern (editor +
test runner + log tail + reference) overflows traditional
fixed-workspace tilers; Niri's scrolling layout absorbs the
overflow.

## Consequences

- Engine documentation that includes screenshots is taken
  under Niri (with the team's standard configuration). A
  contributor on a different compositor will produce slightly
  different screenshots; the deviation is acknowledged in the
  audit's exception register if it ever becomes load-bearing.
- The team's dotfiles (Niri config, terminal config, editor
  config) live in a separate repo, referenced from spec §XVIII
  / `docs/architecture/dev-env.md` (Phase 10+).
- Niri's NVIDIA path is sensitive to driver versions; the
  spec's reference hardware notes the supported driver range
  in `docs/architecture/dev-env.md`.
- No engine code depends on Niri-specific behaviour; the
  engine runs on any compliant Wayland compositor (and on X11
  with `WAYLAND_DISPLAY` unset).

## Risks and tradeoffs

- **Niri is a relatively new project** (vs. Sway's decade of
  maturity). Mitigation: yalter's release cadence is stable;
  the team's experience with Niri in 2026 is solid.
- **NVIDIA + Wayland edge cases.** Mitigation: the
  reference workstation is the test-bed; any Niri regression
  on NVIDIA is reportable upstream.
- **Workflow lock-in.** A team member preferring Sway / GNOME /
  i3 is welcome; their workflow is supported. Niri is the
  reference, not the requirement.

## Alternatives considered

- **Sway.** The previous reference; replaced by Niri per spec
  finding F-02. Sway is the obvious fallback if Niri's
  development pace slows.
- **Hyprland.** Considered; not Rust; not the project's
  idiom; many features the engine doesn't need.
- **No compositor recommendation.** Loses the reference-
  environment reproducibility property; the engine's
  visual-asset captures become per-developer noise. Rejected.
- **A team-local virtual machine.** Heavy; loses the team's
  native GPU access. Rejected.

## Verification

- The spec's `docs/architecture/dev-env.md` (Phase 10+)
  records the canonical Niri configuration and the supported
  NVIDIA driver range.
- Screenshots that ship in engine documentation are taken on
  Niri (annotated as such in the file metadata when material).
- A new team member's onboarding (Phase 10+ onboarding
  writeup) includes the Niri install steps.
- No CI gate; the compositor choice is a developer-environment
  recommendation, not a build-time contract.
