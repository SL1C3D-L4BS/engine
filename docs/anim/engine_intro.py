"""[ENGINE] explainer — starter Manim scene.

Render (high quality → ~/Videos/engine-anim):
    manim-render docs/anim/engine_intro.py EngineIntro
Fast preview:
    manim-render docs/anim/engine_intro.py EngineIntro -q l
Transparent (for an OBS overlay):
    manim-render docs/anim/engine_intro.py EngineIntro -t

The palette mirrors ~/.dotfiles/system/tokens.toml [color] so explainer videos
match the workstation, the Waybar, and the engine UI. One design language,
kernel → compositor → bar → editor → engine → the explainer about the engine.
"""

from manim import (  # noqa: F401
    BOLD,
    DOWN,
    UP,
    Create,
    FadeIn,
    Scene,
    Text,
    Underline,
    Write,
)

# tokens.toml [color]
BG = "#1E1E1E"        # Charcoal Black
FG = "#F7F6F2"        # Soft White
PRIMARY = "#2961B1"   # Royal Blue — engine
SECONDARY = "#64A8E5"  # Sky Blue


class EngineIntro(Scene):
    def construct(self) -> None:
        self.camera.background_color = BG
        title = Text("[ENGINE]", color=FG, weight=BOLD).scale(1.6)
        rule = Underline(title, color=PRIMARY)
        sub = Text("a from-scratch, zero-dependency Rust engine platform", color=SECONDARY)
        sub.scale(0.42).next_to(title, DOWN, buff=0.45)

        self.play(Write(title), run_time=1.2)
        self.play(Create(rule), FadeIn(sub, shift=UP * 0.2))
        self.wait(0.5)
        self.play(title.animate.set_color(PRIMARY), run_time=0.8)
        self.wait(0.8)
