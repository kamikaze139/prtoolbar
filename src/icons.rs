#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
// Pixel maths on a 36 px canvas; the casts are exact.

//! Bitmaps drawn at runtime for the menu bar glyph and avatar masks.
//!
//! The tray image is 36 px tall, which is exactly 18 pt on a Retina display.

use image::{Rgba, RgbaImage};

/// Height (and width, for square icons) of every bitmap in pixels.
pub const ICON_PX: u32 = 36;

const BLACK: [u8; 3] = [0, 0, 0];

/// GitHub's official pull-request mark, as a template image (pure black plus
/// alpha, which macOS recolours to match the menu bar).
///
/// This is a transcription of the `git-pull-request-24` Octicon: the same
/// centre lines and 1.5-unit stroke, scaled from its 24-unit grid onto our
/// 36 px canvas. Octicons are MIT licensed, see `THIRD-PARTY-NOTICES.md`.
pub fn menubar_glyph() -> RgbaImage {
    /// Octicon units to pixels: the icon is designed on a 24-unit grid.
    const K: f32 = ICON_PX as f32 / 24.0;
    /// Stroke width and commit-circle radius, in pixels.
    const STROKE: f32 = 1.5 * K;
    const CIRCLE: f32 = 2.5 * K;

    let at = |x: f32, y: f32| (x * K, y * K);
    let mut canvas = Canvas::new(ICON_PX, ICON_PX);

    // Source branch on the left: two commits joined by a line.
    canvas.ring(at(4.75, 4.75), CIRCLE, STROKE);
    canvas.segment(at(4.75, 8.0), at(4.75, 16.0), STROKE);
    canvas.ring(at(4.75, 19.25), CIRCLE, STROKE);

    // Target branch on the right: down from the corner into a third commit.
    canvas.segment(at(19.25, 7.75), at(19.25, 15.75), STROKE);
    canvas.ring(at(19.25, 19.25), CIRCLE, STROKE);

    // The arm: an arrow head, a run to the right, then a rounded corner down.
    canvas.segment(at(12.875, 2.25), at(10.375, 4.75), STROKE);
    canvas.segment(at(10.375, 4.75), at(12.875, 7.25), STROKE);
    canvas.segment(at(10.375, 4.75), at(16.25, 4.75), STROKE);
    canvas.arc(
        at(16.25, 7.75),
        3.0 * K,
        STROKE,
        -std::f32::consts::FRAC_PI_2,
        0.0,
    );

    canvas.finish(BLACK)
}

/// A monochrome coverage buffer, the canvas the menu bar glyph is drawn into.
///
/// Shapes are rasterised as signed distance fields and combined with `max`,
/// so overlapping strokes join seamlessly instead of darkening each other or
/// leaving seams. Only the alpha channel carries the drawing; the colour is
/// applied once at the end, which is what a macOS template image needs.
struct Canvas {
    width: u32,
    height: u32,
    coverage: Vec<f32>,
}

impl Canvas {
    fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            coverage: vec![0.0; (width * height) as usize],
        }
    }

    /// Add a shape, given a function from pixel centre to distance from the
    /// shape's outline. Negative distance is inside.
    fn add(&mut self, distance: impl Fn(f32, f32) -> f32) {
        for y in 0..self.height {
            for x in 0..self.width {
                let d = distance(x as f32 + 0.5, y as f32 + 0.5);
                let c = (0.5 - d).clamp(0.0, 1.0);
                if c > 0.0 {
                    let slot = &mut self.coverage[(y * self.width + x) as usize];
                    *slot = slot.max(c);
                }
            }
        }
    }

    /// A straight stroke with round caps.
    fn segment(&mut self, (x0, y0): (f32, f32), (x1, y1): (f32, f32), width: f32) {
        let half = width / 2.0;
        let (dx, dy) = (x1 - x0, y1 - y0);
        let len_sq = dx.mul_add(dx, dy * dy);
        self.add(move |px, py| {
            let t = if len_sq == 0.0 {
                0.0
            } else {
                (((px - x0) * dx + (py - y0) * dy) / len_sq).clamp(0.0, 1.0)
            };
            let (nx, ny) = (dx.mul_add(t, x0), dy.mul_add(t, y0));
            (px - nx).hypot(py - ny) - half
        });
    }

    /// A circular arc with round caps, swept clockwise on screen from
    /// `start` to `end` radians (0 points right, pi/2 points down).
    fn arc(&mut self, (cx, cy): (f32, f32), radius: f32, width: f32, start: f32, end: f32) {
        let half = width / 2.0;
        let ends = [start, end].map(|a| (radius.mul_add(a.cos(), cx), radius.mul_add(a.sin(), cy)));
        self.add(move |px, py| {
            let (dx, dy) = (px - cx, py - cy);
            let mut angle = dy.atan2(dx);
            while angle < start {
                angle += std::f32::consts::TAU;
            }
            if angle <= end {
                return (dx.hypot(dy) - radius).abs() - half;
            }
            ends.iter()
                .map(|(ex, ey)| (px - ex).hypot(py - ey) - half)
                .fold(f32::MAX, f32::min)
        });
    }

    /// A circle outline. `radius` is the centre line of the stroke, so the
    /// drawn band runs from `radius - width / 2` to `radius + width / 2`.
    fn ring(&mut self, (cx, cy): (f32, f32), radius: f32, width: f32) {
        let half = width / 2.0;
        // No angular restriction, so there is no seam where a sweep wraps.
        self.add(move |px, py| ((px - cx).hypot(py - cy) - radius).abs() - half);
    }

    /// Apply a colour to the accumulated coverage.
    fn finish(&self, rgb: [u8; 3]) -> RgbaImage {
        let mut img = RgbaImage::new(self.width, self.height);
        for (i, pixel) in img.pixels_mut().enumerate() {
            let alpha = (self.coverage[i] * 255.0).round() as u8;
            *pixel = Rgba([rgb[0], rgb[1], rgb[2], alpha]);
        }
        img
    }
}

/// Anti-aliased filled circle. Used by `avatars` for the circular mask.
pub fn fill_circle(img: &mut RgbaImage, cx: f32, cy: f32, radius: f32, rgb: [u8; 3]) {
    for (x, y, pixel) in img.enumerate_pixels_mut() {
        let dx = x as f32 + 0.5 - cx;
        let dy = y as f32 + 0.5 - cy;
        let coverage = (radius + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
        if coverage > 0.0 {
            let alpha = (coverage * 255.0).round() as u8;
            if pixel.0[3] == 0 || alpha == 0xFF {
                *pixel = Rgba([rgb[0], rgb[1], rgb[2], alpha]);
            } else {
                pixel.0[3] = pixel.0[3].max(alpha);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three commit circles are rings, not discs: ink on the band, a
    /// transparent hole at the centre. This pins the octicon's geometry, so
    /// an accidental edit to the coordinates fails here rather than shipping.
    #[test]
    fn menubar_glyph_draws_three_open_commit_circles() {
        let glyph = menubar_glyph();
        let k = ICON_PX as f32 / 24.0;
        // Round rather than truncate: 19.25 units is 28.875 px, and 28 would
        // sample the hole instead of the ring.
        let px = |units: f32| (units * k).round() as u32;
        for (x, y) in [(4.75, 4.75), (4.75, 19.25), (19.25, 19.25)] {
            let (cx, cy) = (px(x), px(y));
            assert_eq!(
                glyph.get_pixel(cx, cy).0[3],
                0,
                "circle at ({x}, {y}) is open in the middle"
            );
            // The stroke sits about 2.5 units out. Take the strongest pixel
            // across that band rather than one sample, which can land on a
            // partially covered edge pixel depending on sub-pixel placement.
            let band = (cx + px(1.8)..=cx + px(3.2))
                .map(|x| glyph.get_pixel(x, cy).0[3])
                .max()
                .unwrap();
            assert!(
                band > 200,
                "circle at ({x}, {y}) has an inked band, got {band}"
            );
        }
    }

    #[test]
    fn menubar_glyph_is_a_black_template_image() {
        let glyph = menubar_glyph();
        assert_eq!((glyph.width(), glyph.height()), (ICON_PX, ICON_PX));
        let opaque = glyph.pixels().filter(|p| p.0[3] > 0).count();
        assert!(opaque > 50, "glyph has visible pixels, got {opaque}");
        assert!(
            glyph
                .pixels()
                .all(|p| p.0[0] == 0 && p.0[1] == 0 && p.0[2] == 0),
            "template images must be pure black plus alpha"
        );
    }
}
