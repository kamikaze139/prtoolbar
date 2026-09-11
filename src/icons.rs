#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
// Pixel maths on 36 px canvases; the casts are exact.

//! Bitmaps drawn at runtime: status dots and the menu bar glyph.
//!
//! Menu and tray images are 36 px tall. `muda` and `tray-icon` scale them to
//! 18 pt on macOS. The 18 px reviewer badges are composited into these images.

use image::{Rgba, RgbaImage};

use crate::model::{ReviewState, Status};

/// Height (and width, for square icons) of every bitmap in pixels.
pub const ICON_PX: u32 = 36;

const GREEN: [u8; 3] = [0x34, 0xC7, 0x59];
const RED: [u8; 3] = [0xFF, 0x3B, 0x30];
const YELLOW: [u8; 3] = [0xFF, 0xCC, 0x00];
const GREY: [u8; 3] = [0x8E, 0x8E, 0x93];
const BLACK: [u8; 3] = [0, 0, 0];

/// A 20 px coloured circle centred on a transparent 36 px canvas.
pub fn status_dot(status: Status) -> RgbaImage {
    let rgb = match status {
        Status::Ready => GREEN,
        Status::Blocked => RED,
        Status::Waiting => YELLOW,
        Status::Draft => GREY,
    };
    let mut img = RgbaImage::new(ICON_PX, ICON_PX);
    let centre = ICON_PX as f32 / 2.0;
    fill_circle(&mut img, centre, centre, 10.0, rgb);
    img
}

/// Small GitHub-style reviewer badge: check, cross, comment, or pending clock.
pub fn review_badge(state: ReviewState) -> RgbaImage {
    let (colour, glyph) = match state {
        ReviewState::Approved => (GREEN, ["     ", "    #", "#  # ", " ##  ", "     "]),
        ReviewState::ChangesRequested => (RED, ["#   #", " # # ", "  #  ", " # # ", "#   #"]),
        ReviewState::Commented => (GREY, ["#####", "#   #", "#   #", "#####", " #   "]),
        ReviewState::Pending => (YELLOW, ["  #  ", "  #  ", "  ###", "     ", "     "]),
    };
    let mut badge = RgbaImage::new(18, 18);
    fill_circle(&mut badge, 9.0, 9.0, 9.0, colour);
    let ink = if state == ReviewState::Pending {
        BLACK
    } else {
        [255, 255, 255]
    };
    for (y, row) in glyph.iter().enumerate() {
        for (x, pixel) in row.bytes().enumerate() {
            if pixel == b'#' {
                fill_rect(&mut badge, 4 + x as u32 * 2, 4 + y as u32 * 2, 2, 2, ink);
            }
        }
    }
    badge
}

/// A monochrome pull-request glyph (two branch dots joined by a line, with a
/// merge arm) for use as a template image in the menu bar.
pub fn menubar_glyph() -> RgbaImage {
    let mut img = RgbaImage::new(ICON_PX, ICON_PX);
    // Left column: top dot, vertical line, bottom dot.
    ring(&mut img, 9.0, 8.0, 5.0, 2.5);
    fill_rect(&mut img, 8, 13, 2, 11, BLACK);
    ring(&mut img, 9.0, 28.0, 5.0, 2.5);
    // Right column: arm coming from the top-left, going down to a dot.
    fill_rect(&mut img, 14, 7, 9, 2, BLACK);
    fill_rect(&mut img, 26, 7, 2, 16, BLACK);
    ring(&mut img, 27.0, 28.0, 5.0, 2.5);
    // Small arrow head at the end of the arm.
    fill_rect(&mut img, 22, 4, 2, 2, BLACK);
    fill_rect(&mut img, 22, 10, 2, 2, BLACK);
    img
}

/// Anti-aliased filled circle.
pub fn fill_circle(img: &mut RgbaImage, cx: f32, cy: f32, radius: f32, rgb: [u8; 3]) {
    for (x, y, pixel) in img.enumerate_pixels_mut() {
        let dx = x as f32 + 0.5 - cx;
        let dy = y as f32 + 0.5 - cy;
        let coverage = (radius + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
        if coverage > 0.0 {
            blend_same_colour(pixel, rgb, coverage);
        }
    }
}

/// Anti-aliased ring (circle outline) with the given stroke width.
fn ring(img: &mut RgbaImage, cx: f32, cy: f32, radius: f32, stroke: f32) {
    let outer = radius;
    let inner = radius - stroke;
    for (x, y, pixel) in img.enumerate_pixels_mut() {
        let dx = x as f32 + 0.5 - cx;
        let dy = y as f32 + 0.5 - cy;
        let d = (dx * dx + dy * dy).sqrt();
        let coverage = (outer + 0.5 - d).clamp(0.0, 1.0) * (d - inner + 0.5).clamp(0.0, 1.0);
        if coverage > 0.0 {
            blend_same_colour(pixel, BLACK, coverage);
        }
    }
}

fn fill_rect(img: &mut RgbaImage, x0: u32, y0: u32, w: u32, h: u32, rgb: [u8; 3]) {
    for y in y0..(y0 + h).min(img.height()) {
        for x in x0..(x0 + w).min(img.width()) {
            *img.get_pixel_mut(x, y) = Rgba([rgb[0], rgb[1], rgb[2], 0xFF]);
        }
    }
}

/// Alpha-blend `rgb` at `coverage` over an existing pixel. Only supports one
/// colour per canvas: repeated calls with different colours on the same
/// pixel do not composite correctly, since alpha is maxed rather than
/// accumulated.
fn blend_same_colour(pixel: &mut Rgba<u8>, rgb: [u8; 3], coverage: f32) {
    let alpha = (coverage * 255.0).round() as u8;
    if pixel.0[3] == 0 || alpha == 0xFF {
        *pixel = Rgba([rgb[0], rgb[1], rgb[2], alpha]);
    } else {
        pixel.0[3] = pixel.0[3].max(alpha);
    }
}

/// Convert to the status item icon type.
pub fn tray_icon(img: &RgbaImage) -> tray_icon::Icon {
    tray_icon::Icon::from_rgba(img.as_raw().clone(), img.width(), img.height())
        .expect("RGBA buffer matches its dimensions")
}

/// Convert to the menu item icon type.
pub fn menu_icon(img: &RgbaImage) -> tray_icon::menu::Icon {
    tray_icon::menu::Icon::from_rgba(img.as_raw().clone(), img.width(), img.height())
        .expect("RGBA buffer matches its dimensions")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Status;

    #[test]
    fn status_dot_is_coloured_in_the_middle_and_clear_in_the_corner() {
        let dot = status_dot(Status::Ready);
        assert_eq!((dot.width(), dot.height()), (ICON_PX, ICON_PX));
        let centre = dot.get_pixel(ICON_PX / 2, ICON_PX / 2).0;
        assert_eq!(centre, [0x34, 0xC7, 0x59, 0xFF]);
        assert_eq!(dot.get_pixel(0, 0).0[3], 0, "corner is transparent");
    }

    #[test]
    fn every_status_has_a_distinct_colour() {
        let colours: Vec<[u8; 4]> = [
            Status::Ready,
            Status::Blocked,
            Status::Waiting,
            Status::Draft,
        ]
        .iter()
        .map(|s| status_dot(*s).get_pixel(ICON_PX / 2, ICON_PX / 2).0)
        .collect();
        for (i, a) in colours.iter().enumerate() {
            for b in &colours[i + 1..] {
                assert_ne!(a, b);
            }
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

    #[test]
    fn conversions_do_not_panic() {
        let dot = status_dot(Status::Waiting);
        let _tray = tray_icon(&dot);
        let _menu = menu_icon(&dot);
    }
}
