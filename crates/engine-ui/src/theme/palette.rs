// GENERATED from ~/.dotfiles/system/tokens.toml by `render-engine-palette`.
// DO NOT EDIT — edit tokens.toml then run `just gen-palette`.
#![allow(dead_code)]

/// 8-bit sRGB colour with alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba {
    /// Red channel (0-255).
    pub r: u8,
    /// Green channel (0-255).
    pub g: u8,
    /// Blue channel (0-255).
    pub b: u8,
    /// Alpha channel (0-255).
    pub a: u8,
}

impl Rgba {
    /// Opaque colour from a packed `0xRRGGBB` literal.
    pub const fn rgb(v: u32) -> Self {
        Self {
            r: (v >> 16) as u8,
            g: (v >> 8) as u8,
            b: v as u8,
            a: 255,
        }
    }
}

/// Semantic colours (tokens.toml `[color]`).
pub mod color {
    use super::Rgba;
    /// `#1E1E1E`
    pub const BG: Rgba = Rgba::rgb(0x1E1E1E);
    /// `#2B2B2B`
    pub const BG_ALT: Rgba = Rgba::rgb(0x2B2B2B);
    /// `#3A3A3A`
    pub const BG_ALT_LIGHT: Rgba = Rgba::rgb(0x3A3A3A);
    /// `#F7F6F2`
    pub const FG: Rgba = Rgba::rgb(0xF7F6F2);
    /// `#B8A789`
    pub const FG_MUTED: Rgba = Rgba::rgb(0xB8A789);
    /// `#2961B1`
    pub const PRIMARY: Rgba = Rgba::rgb(0x2961B1);
    /// `#64A8E5`
    pub const SECONDARY: Rgba = Rgba::rgb(0x64A8E5);
    /// `#D9892B`
    pub const TERTIARY: Rgba = Rgba::rgb(0xD9892B);
    /// `#D95C5C`
    pub const ERROR: Rgba = Rgba::rgb(0xD95C5C);
    /// `#C7D42B`
    pub const SUCCESS: Rgba = Rgba::rgb(0xC7D42B);
    /// `#5EAFC9`
    pub const CODING: Rgba = Rgba::rgb(0x5EAFC9);
    /// `#9FB94F`
    pub const BROWSER: Rgba = Rgba::rgb(0x9FB94F);
    /// `#7C6ED6`
    pub const MEDIA: Rgba = Rgba::rgb(0x7C6ED6);
    /// `#2FA39A`
    pub const NET: Rgba = Rgba::rgb(0x2FA39A);
    /// `#C264B0`
    pub const AI: Rgba = Rgba::rgb(0xC264B0);
    /// `#D98AAE`
    pub const AGENDA: Rgba = Rgba::rgb(0xD98AAE);
    /// `#1A3D6F`
    pub const SELECTION: Rgba = Rgba::rgb(0x1A3D6F);
    /// `#3D3528`
    pub const INACTIVE: Rgba = Rgba::rgb(0x3D3528);
    /// `#1A1612`
    pub const WARM_SHADOW: Rgba = Rgba::rgb(0x1A1612);
    /// `#BED7F4`
    pub const MIST: Rgba = Rgba::rgb(0xBED7F4);
    /// `#D3C399`
    pub const SAND: Rgba = Rgba::rgb(0xD3C399);
    /// `#7A6142`
    pub const EARTH: Rgba = Rgba::rgb(0x7A6142);
    /// `#7C6ED6`
    pub const WISTERIA: Rgba = Rgba::rgb(0x7C6ED6);
    /// `#D6BF3A`
    pub const HONEY: Rgba = Rgba::rgb(0xD6BF3A);
    /// `#8E3A32`
    pub const BRICK: Rgba = Rgba::rgb(0x8E3A32);
}

/// Nerd-Font glyphs (tokens.toml `[icon]`).
pub mod icon {
    /// `✓`
    pub const OK: &str = "✓";
    /// `✗`
    pub const FAIL: &str = "✗";
    /// `·`
    pub const IDLE: &str = "·";
    /// `󰋽`
    pub const INFO: &str = "󰋽";
    /// `󰀦`
    pub const ALERT: &str = "󰀦";
    /// `󰀪`
    pub const CRITICAL: &str = "󰀪";
    /// `󱥸`
    pub const BUILDING: &str = "󱥸";
    /// `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏`
    pub const SPINNER: &str = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏";
    /// `󰒋`
    pub const ENGINE: &str = "󰒋";
    /// `󰭹`
    pub const COMMS: &str = "󰭹";
    /// `󰕧`
    pub const STREAM: &str = "󰕧";
    /// `󰓅`
    pub const MONITORING: &str = "󰓅";
    /// `󰊴`
    pub const GAMING: &str = "󰊴";
    /// `󰨸`
    pub const CODING: &str = "󰨸";
    /// `󰈹`
    pub const BROWSER: &str = "󰈹";
    /// `󰎆`
    pub const MEDIA: &str = "󰎆";
    /// `󰛳`
    pub const NET: &str = "󰛳";
    /// `󰚩`
    pub const AI: &str = "󰚩";
    /// `󰃭`
    pub const AGENDA: &str = "󰃭";
    /// `󰥔`
    pub const CLOCK: &str = "󰥔";
    /// `󰻠`
    pub const CPU: &str = "󰻠";
    /// `󰍛`
    pub const MEM: &str = "󰍛";
    /// `󰔏`
    pub const TEMP: &str = "󰔏";
    /// `󰋊`
    pub const DISK: &str = "󰋊";
    /// `󰇚`
    pub const NET_DOWN: &str = "󰇚";
    /// `󰕒`
    pub const NET_UP: &str = "󰕒";
    /// `󰍬`
    pub const MIC: &str = "󰍬";
    /// `󰍭`
    pub const MUTE: &str = "󰍭";
    /// `⏺`
    pub const REC: &str = "⏺";
    /// `󰐰`
    pub const LIVE: &str = "󰐰";
    /// `󰢮`
    pub const GPU: &str = "󰢮";
    /// `󰁹`
    pub const BATTERY: &str = "󰁹";
    /// `󰂚`
    pub const BELL: &str = "󰂚";
    /// `󰇮`
    pub const MAIL: &str = "󰇮";
    /// `󰦝`
    pub const VPN: &str = "󰦝";
    /// `󰌾`
    pub const LOCK: &str = "󰌾";
    /// `󰊢`
    pub const GIT: &str = "󰊢";
    /// `󰔛`
    pub const POMO: &str = "󰔛";
    /// `󰉁`
    pub const SPEND: &str = "󰉁";
    /// `󰐊`
    pub const PLAY: &str = "󰐊";
    /// `󰏤`
    pub const PAUSE: &str = "󰏤";
    /// `󰕾`
    pub const SPEAKER: &str = "󰕾";
    /// `󰞀`
    pub const FIREWALL: &str = "󰞀";
    /// `󰑪`
    pub const ROUTE: &str = "󰑪";
    /// `🦀`
    pub const RUST: &str = "🦀";
}

/// Geometry in logical px (tokens.toml `[geom]`).
pub mod geom {
    /// `4` px
    pub const UNIT: u32 = 4;
    /// `8` px
    pub const GAP: u32 = 8;
    /// `16` px
    pub const GAP_LG: u32 = 16;
    /// `24` px
    pub const GAP_XL: u32 = 24;
    /// `40` px
    pub const BAR_HEIGHT: u32 = 40;
    /// `32` px
    pub const BAR_HEIGHT_COMPACT: u32 = 32;
    /// `32` px
    pub const CHIP_HEIGHT: u32 = 32;
    /// `2` px
    pub const PAD_Y: u32 = 2;
    /// `12` px
    pub const PAD_X: u32 = 12;
    /// `16` px
    pub const PAD_X_LG: u32 = 16;
    /// `4` px
    pub const RADIUS_SM: u32 = 4;
    /// `6` px
    pub const RADIUS: u32 = 6;
    /// `12` px
    pub const RADIUS_LG: u32 = 12;
    /// `14` px
    pub const RADIUS_WINDOW: u32 = 14;
    /// `2` px
    pub const BORDER: u32 = 2;
    /// `4` px
    pub const BORDER_LG: u32 = 4;
}

/// Motion durations in ms + easing curve (tokens.toml `[motion]`).
pub mod motion {
    /// `120` ms
    pub const QUICK_MS: u32 = 120;
    /// `200` ms
    pub const NORMAL_MS: u32 = 200;
    /// `380` ms
    pub const SLOW_MS: u32 = 380;
    /// `2400` ms
    pub const PULSE_MS: u32 = 2400;
    /// `120` ms
    pub const LIFT_MS: u32 = 120;
    /// `cubic-bezier(0.2, 0.8, 0.2, 1.0)`
    pub const EASE: &str = "cubic-bezier(0.2, 0.8, 0.2, 1.0)";
}
