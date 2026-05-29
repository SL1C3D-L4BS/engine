# [ENGINE] explainer animations (Manim)

Mathematical/architectural explainer animations for [ENGINE], rendered with
[Manim](https://www.manim.community/). Scenes live here; the render helper lives
in the dotfiles (`~/.local/bin/manim-render`).

## Render

```bash
manim-render docs/anim/engine_intro.py EngineIntro        # high quality
manim-render docs/anim/engine_intro.py EngineIntro -q l   # fast preview
manim-render docs/anim/engine_intro.py EngineIntro -t     # transparent (OBS overlay)
```

Output goes to `~/Videos/engine-anim/` (override with `$MANIM_OUT`), **never**
into the repo — so videos are not committed.

## Design language

Scenes hardcode the palette hexes from `~/.dotfiles/system/tokens.toml` `[color]`
(Royal Blue `#2961B1` = engine, Soft White `#F7F6F2` = fg, Charcoal `#1E1E1E` =
bg) so explainer videos match the Waybar, Niri, and the engine UI — one design
language all the way through. Keep new scenes on the same palette.

## Toolchain

`manim` (uv tool) + system `ffmpeg`, `cairo`, `pango`, `texlive` (for LaTeX
`Tex`/`MathTex`). Companion media tooling on the workstation: `typst` (docs),
`d2` + `mermaid` (`mmdc`) (diagrams), `vhs` (terminal-session GIFs), `asciinema`
(terminal casts).
