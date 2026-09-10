#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

//! Reviewer avatars: download, circular mask, and side-by-side strips.

use std::collections::HashMap;

use anyhow::{Context, Result};
use image::imageops::{self, FilterType};
use image::{Rgba, RgbaImage};
use ureq::Agent;

use crate::icons::{ICON_PX, fill_circle};

/// Diameter of one avatar in pixels (renders at 16 pt).
pub const AVATAR_PX: u32 = 32;
/// Horizontal gap between avatars in a strip.
pub const GAP_PX: u32 = 6;
/// Avatars shown per PR before the label falls back to `+N`.
pub const MAX_AVATARS: usize = 5;

const GREY: [u8; 3] = [0x8E, 0x8E, 0x93];

/// Process-lifetime cache of decoded, masked avatars keyed by URL.
#[derive(Debug, Default)]
pub struct AvatarCache {
    images: HashMap<String, RgbaImage>,
}

impl AvatarCache {
    /// Fetch every URL not yet cached and return only those new entries.
    ///
    /// Failures are logged and cached as a placeholder so they are not retried
    /// on every refresh. Empty URLs are ignored.
    pub fn fetch_missing(
        &mut self,
        agent: &Agent,
        urls: impl IntoIterator<Item = String>,
    ) -> HashMap<String, RgbaImage> {
        let mut fresh = HashMap::new();
        for url in urls {
            if url.is_empty() || self.images.contains_key(&url) || fresh.contains_key(&url) {
                continue;
            }
            let image = match fetch(agent, &url) {
                Ok(image) => image,
                Err(err) => {
                    log::warn!("avatar {url}: {err:#}");
                    placeholder()
                }
            };
            fresh.insert(url, image);
        }
        self.images
            .extend(fresh.iter().map(|(k, v)| (k.clone(), v.clone())));
        fresh
    }
}

fn fetch(agent: &Agent, url: &str) -> Result<RgbaImage> {
    let bytes = agent
        .get(url)
        .call()
        .context("request failed")?
        .body_mut()
        .read_to_vec()
        .context("reading body")?;
    decode(&bytes)
}

/// Decode any supported image, resize to [`AVATAR_PX`] and apply the mask.
pub fn decode(bytes: &[u8]) -> Result<RgbaImage> {
    let decoded = image::load_from_memory(bytes).context("not a supported image")?;
    let resized = imageops::resize(&decoded, AVATAR_PX, AVATAR_PX, FilterType::Triangle);
    Ok(circle_mask(resized))
}

/// Grey circle used when an avatar is missing or failed to load.
pub fn placeholder() -> RgbaImage {
    let mut img = RgbaImage::new(AVATAR_PX, AVATAR_PX);
    let centre = AVATAR_PX as f32 / 2.0;
    fill_circle(&mut img, centre, centre, centre, GREY);
    img
}

/// Keep only the pixels inside the inscribed circle, with a soft edge.
pub fn circle_mask(mut img: RgbaImage) -> RgbaImage {
    let radius = img.width().min(img.height()) as f32 / 2.0;
    let (cx, cy) = (img.width() as f32 / 2.0, img.height() as f32 / 2.0);
    for (x, y, pixel) in img.enumerate_pixels_mut() {
        let dx = x as f32 + 0.5 - cx;
        let dy = y as f32 + 0.5 - cy;
        let coverage = (radius + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
        let alpha = (f32::from(pixel.0[3]) * coverage).round() as u8;
        *pixel = Rgba([pixel.0[0], pixel.0[1], pixel.0[2], alpha]);
    }
    img
}

/// Lay avatars out left to right on a transparent [`ICON_PX`]-tall canvas.
///
/// At most [`MAX_AVATARS`] are drawn. An empty input yields a 1 px wide
/// transparent image so callers never have to special-case it.
pub fn compose_strip(avatars: &[&RgbaImage]) -> RgbaImage {
    let shown = &avatars[..avatars.len().min(MAX_AVATARS)];
    if shown.is_empty() {
        return RgbaImage::new(1, ICON_PX);
    }
    let n = shown.len() as u32;
    let width = n * AVATAR_PX + (n - 1) * GAP_PX;
    let mut strip = RgbaImage::new(width, ICON_PX);
    let y = i64::from((ICON_PX - AVATAR_PX) / 2);
    for (i, avatar) in shown.iter().enumerate() {
        let x = i64::from(i as u32 * (AVATAR_PX + GAP_PX));
        imageops::overlay(&mut strip, *avatar, x, y);
    }
    strip
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn solid(rgb: [u8; 3]) -> RgbaImage {
        RgbaImage::from_pixel(AVATAR_PX, AVATAR_PX, Rgba([rgb[0], rgb[1], rgb[2], 0xFF]))
    }

    #[test]
    fn circle_mask_clears_corners_and_keeps_centre() {
        let masked = circle_mask(solid([10, 20, 30]));
        assert_eq!(masked.get_pixel(0, 0).0[3], 0);
        assert_eq!(masked.get_pixel(AVATAR_PX - 1, AVATAR_PX - 1).0[3], 0);
        assert_eq!(
            masked.get_pixel(AVATAR_PX / 2, AVATAR_PX / 2).0,
            [10, 20, 30, 0xFF]
        );
    }

    #[test]
    fn strip_dimensions_follow_avatar_count() {
        let a = solid([1, 1, 1]);
        let b = solid([2, 2, 2]);
        let strip = compose_strip(&[&a, &b]);
        assert_eq!(strip.height(), ICON_PX);
        assert_eq!(strip.width(), 2 * AVATAR_PX + GAP_PX);
        // First avatar starts at x = 0, second after avatar + gap; both vertically centred.
        let y = ICON_PX / 2;
        assert_eq!(strip.get_pixel(AVATAR_PX / 2, y).0, [1, 1, 1, 0xFF]);
        assert_eq!(
            strip.get_pixel(AVATAR_PX + GAP_PX + AVATAR_PX / 2, y).0,
            [2, 2, 2, 0xFF]
        );
        assert_eq!(
            strip.get_pixel(AVATAR_PX + GAP_PX / 2, y).0[3],
            0,
            "gap is transparent"
        );
    }

    #[test]
    fn strip_caps_at_max_avatars() {
        let a = solid([1, 1, 1]);
        let many: Vec<&RgbaImage> = std::iter::repeat_n(&a, MAX_AVATARS + 3).collect();
        let strip = compose_strip(&many);
        let n = MAX_AVATARS as u32;
        assert_eq!(strip.width(), n * AVATAR_PX + (n - 1) * GAP_PX);
    }

    #[test]
    fn empty_strip_is_a_single_transparent_column() {
        let strip = compose_strip(&[]);
        assert_eq!((strip.width(), strip.height()), (1, ICON_PX));
    }

    #[test]
    fn decode_resizes_and_masks_a_png() {
        let big = RgbaImage::from_pixel(64, 64, Rgba([200, 100, 50, 0xFF]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        big.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let avatar = decode(bytes.get_ref()).unwrap();
        assert_eq!((avatar.width(), avatar.height()), (AVATAR_PX, AVATAR_PX));
        assert_eq!(
            avatar.get_pixel(AVATAR_PX / 2, AVATAR_PX / 2).0,
            [200, 100, 50, 0xFF]
        );
        assert_eq!(avatar.get_pixel(0, 0).0[3], 0);
    }

    #[test]
    fn decode_rejects_garbage() {
        assert!(decode(b"not an image").is_err());
    }

    #[test]
    fn placeholder_is_grey_and_round() {
        let p = placeholder();
        assert_eq!((p.width(), p.height()), (AVATAR_PX, AVATAR_PX));
        assert_eq!(
            p.get_pixel(AVATAR_PX / 2, AVATAR_PX / 2).0,
            [0x8E, 0x8E, 0x93, 0xFF]
        );
        assert_eq!(p.get_pixel(0, 0).0[3], 0);
    }
}
