//! Framebuffer + Z-buffer + linear/sRGB conversion (ADR-046 §sRGB-aware).
//!
//! The rasterizer writes sRGB-encoded RGBA8 pixels via the `Framebuffer`
//! API. Internally, shading happens in *linear* space and the final
//! store converts to sRGB. The oracle's pixel comparison (ADR-046)
//! decodes both sides back to linear before differencing, which is the
//! "sRGB-aware" property the ADR requires.

/// One 8-bit-per-channel sRGB pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rgba8 {
    /// Red channel, sRGB-encoded.
    pub r: u8,
    /// Green channel, sRGB-encoded.
    pub g: u8,
    /// Blue channel, sRGB-encoded.
    pub b: u8,
    /// Alpha channel, linear (alpha is never sRGB-encoded).
    pub a: u8,
}

impl Rgba8 {
    /// Pack from linear RGBA floats in [0, 1]. Clamps + sRGB-encodes
    /// R, G, B; A is stored linearly.
    pub fn from_linear(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self {
            r: linear_to_srgb_byte(r),
            g: linear_to_srgb_byte(g),
            b: linear_to_srgb_byte(b),
            a: clamp01(a).mul_add(255.0, 0.5) as u8,
        }
    }

    /// Decode to linear RGBA floats in [0, 1].
    pub fn to_linear(self) -> (f32, f32, f32, f32) {
        (
            srgb_byte_to_linear(self.r),
            srgb_byte_to_linear(self.g),
            srgb_byte_to_linear(self.b),
            (self.a as f32) / 255.0,
        )
    }
}

/// Convert a single sRGB-encoded byte to linear [0, 1].
pub fn srgb_byte_to_linear(b: u8) -> f32 {
    let x = (b as f32) / 255.0;
    if x <= 0.04045 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}

/// Convert linear [0, 1] to an sRGB-encoded byte, clamping.
pub fn linear_to_srgb_byte(x: f32) -> u8 {
    let x = clamp01(x);
    let y = if x <= 0.003_130_8 {
        x * 12.92
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    };
    y.mul_add(255.0, 0.5).clamp(0.0, 255.0) as u8
}

#[inline]
fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

/// One render target: colour buffer + Z buffer at a fixed extent.
#[derive(Clone, Debug)]
pub struct Framebuffer {
    width: u32,
    height: u32,
    color: Vec<Rgba8>,
    depth: Vec<f32>,
}

impl Framebuffer {
    /// Allocate a new framebuffer of `width × height` pixels. Initial
    /// state: cleared to opaque black, depth = +inf.
    pub fn new(width: u32, height: u32) -> Self {
        let n = (width as usize) * (height as usize);
        Self {
            width,
            height,
            color: vec![Rgba8::default(); n],
            depth: vec![f32::INFINITY; n],
        }
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Borrow the colour buffer.
    pub fn color(&self) -> &[Rgba8] {
        &self.color
    }

    /// Mutably borrow the colour buffer.
    pub fn color_mut(&mut self) -> &mut [Rgba8] {
        &mut self.color
    }

    /// Borrow the depth buffer.
    pub fn depth(&self) -> &[f32] {
        &self.depth
    }

    /// Sample a colour pixel.
    #[inline]
    pub fn sample(&self, x: u32, y: u32) -> Rgba8 {
        self.color[self.idx(x, y)]
    }

    /// Write a colour pixel without bounds checking.
    #[inline]
    pub fn write(&mut self, x: u32, y: u32, c: Rgba8) {
        let i = self.idx(x, y);
        self.color[i] = c;
    }

    /// Sample a depth value.
    #[inline]
    pub fn depth_at(&self, x: u32, y: u32) -> f32 {
        self.depth[self.idx(x, y)]
    }

    /// Conditionally update colour + depth based on depth-less-than.
    #[inline]
    pub fn write_if_closer(&mut self, x: u32, y: u32, z: f32, c: Rgba8) -> bool {
        let i = self.idx(x, y);
        if z < self.depth[i] {
            self.depth[i] = z;
            self.color[i] = c;
            true
        } else {
            false
        }
    }

    /// Clear both colour and depth.
    pub fn clear(&mut self, c: Rgba8, z: f32) {
        for px in &mut self.color {
            *px = c;
        }
        for d in &mut self.depth {
            *d = z;
        }
    }

    #[inline]
    fn idx(&self, x: u32, y: u32) -> usize {
        debug_assert!(x < self.width && y < self.height);
        (y as usize) * (self.width as usize) + (x as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_round_trip_extremes() {
        // 0 and 255 are fixed points modulo float rounding.
        assert_eq!(linear_to_srgb_byte(0.0), 0);
        assert_eq!(linear_to_srgb_byte(1.0), 255);
    }

    #[test]
    fn srgb_mid_value_decode_then_encode_is_idempotent() {
        for b in [16u8, 64, 128, 200, 240] {
            let lin = srgb_byte_to_linear(b);
            let back = linear_to_srgb_byte(lin);
            assert!(
                (back as i32 - b as i32).abs() <= 1,
                "round-trip drift for {b}: got {back}"
            );
        }
    }

    #[test]
    fn write_if_closer_respects_depth() {
        let mut fb = Framebuffer::new(4, 4);
        let red = Rgba8 {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        };
        let blue = Rgba8 {
            r: 0,
            g: 0,
            b: 255,
            a: 255,
        };
        assert!(fb.write_if_closer(2, 2, 0.5, red));
        assert!(!fb.write_if_closer(2, 2, 0.9, blue), "depth=0.9 farther");
        assert_eq!(fb.sample(2, 2), red);
        assert!(fb.write_if_closer(2, 2, 0.1, blue), "depth=0.1 closer");
        assert_eq!(fb.sample(2, 2), blue);
    }

    #[test]
    fn clear_resets_color_and_depth() {
        let mut fb = Framebuffer::new(2, 2);
        let c = Rgba8 {
            r: 10,
            g: 20,
            b: 30,
            a: 255,
        };
        fb.clear(c, 1.0);
        for y in 0..2 {
            for x in 0..2 {
                assert_eq!(fb.sample(x, y), c);
                assert_eq!(fb.depth_at(x, y), 1.0);
            }
        }
    }
}
