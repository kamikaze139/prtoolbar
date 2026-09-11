#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
// Pixel maths on 36 px canvases; the casts are exact.

//! Reviewer avatars: download, cache and circular masks for the popup.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use image::imageops::{self, FilterType};
use image::{Rgba, RgbaImage};
use ureq::Agent;

use crate::icons::fill_circle;

/// Diameter of one avatar in pixels (renders at 16 pt).
pub const AVATAR_PX: u32 = 32;
/// How long a failed URL is skipped before being retried.
pub const RETRY_AFTER: Duration = Duration::from_secs(600);

const GREY: [u8; 3] = [0x8E, 0x8E, 0x93];

/// Process-lifetime cache of decoded, masked avatars keyed by URL.
#[derive(Debug, Default)]
pub struct AvatarCache {
    images: HashMap<String, RgbaImage>,
    /// URLs that failed to fetch, with the time of the last failure.
    failed: HashMap<String, Instant>,
}

impl AvatarCache {
    /// Fetch every URL not yet cached and return only those new entries.
    ///
    /// Failures are logged and skipped on later calls for [`RETRY_AFTER`] so
    /// they are not retried on every refresh, but they are not cached
    /// forever either. Empty URLs are ignored.
    pub fn fetch_missing(
        &mut self,
        agent: &Agent,
        urls: impl IntoIterator<Item = String>,
    ) -> HashMap<String, RgbaImage> {
        self.fetch_missing_with(urls, |url| fetch(agent, url))
    }

    /// Core of [`Self::fetch_missing`], parameterised over the fetch
    /// function so it can be unit-tested without the network.
    pub fn fetch_missing_with(
        &mut self,
        urls: impl IntoIterator<Item = String>,
        mut fetch: impl FnMut(&str) -> Result<RgbaImage>,
    ) -> HashMap<String, RgbaImage> {
        let mut fresh = HashMap::new();
        for url in urls {
            if url.is_empty() || self.images.contains_key(&url) || fresh.contains_key(&url) {
                continue;
            }
            if self
                .failed
                .get(&url)
                .is_some_and(|failed_at| failed_at.elapsed() < RETRY_AFTER)
            {
                continue;
            }
            match fetch(&url) {
                Ok(image) => {
                    self.failed.remove(&url);
                    self.images.insert(url.clone(), image.clone());
                    fresh.insert(url, image);
                }
                Err(err) => {
                    log::warn!("avatar {url}: {err:#}");
                    self.failed.insert(url.clone(), Instant::now());
                    fresh.insert(url, placeholder());
                }
            }
        }
        fresh
    }

    /// Push every failure timestamp `by` further into the past, so
    /// [`RETRY_AFTER`] can be exercised without sleeping in tests.
    #[cfg(test)]
    fn age_failures(&mut self, by: Duration) {
        for instant in self.failed.values_mut() {
            *instant = instant.checked_sub(by).unwrap_or(*instant);
        }
    }
}

fn fetch(agent: &Agent, url: &str) -> Result<RgbaImage> {
    let mut response = agent.get(url).call().context("request failed")?;
    let status = response.status();
    if !status.is_success() {
        return Err(anyhow!("HTTP {status}"));
    }
    let bytes = response.body_mut().read_to_vec().context("reading body")?;
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

    #[test]
    fn already_cached_url_is_not_fetched_again() {
        let mut cache = AvatarCache::default();
        let calls = std::cell::Cell::new(0);
        let fetch = |_: &str| {
            calls.set(calls.get() + 1);
            Ok(solid([1, 1, 1]))
        };
        cache.fetch_missing_with(["https://a".to_owned()], fetch);
        assert_eq!(calls.get(), 1);

        let fresh = cache.fetch_missing_with(["https://a".to_owned()], fetch);
        assert_eq!(calls.get(), 1, "cached URL is not fetched again");
        assert!(fresh.is_empty(), "cached URL is not returned as fresh");
    }

    #[test]
    fn duplicate_url_within_one_call_is_fetched_once() {
        let mut cache = AvatarCache::default();
        let calls = std::cell::Cell::new(0);
        let fetch = |_: &str| {
            calls.set(calls.get() + 1);
            Ok(solid([1, 1, 1]))
        };
        let fresh =
            cache.fetch_missing_with(["https://a".to_owned(), "https://a".to_owned()], fetch);
        assert_eq!(calls.get(), 1);
        assert_eq!(fresh.len(), 1);
    }

    #[test]
    fn empty_url_is_ignored() {
        let mut cache = AvatarCache::default();
        let calls = std::cell::Cell::new(0);
        let fetch = |_: &str| {
            calls.set(calls.get() + 1);
            Ok(solid([1, 1, 1]))
        };
        let fresh = cache.fetch_missing_with([String::new()], fetch);
        assert_eq!(calls.get(), 0);
        assert!(fresh.is_empty());
    }

    #[test]
    fn failing_fetch_yields_placeholder_and_is_not_retried_immediately() {
        let mut cache = AvatarCache::default();
        let calls = std::cell::Cell::new(0);
        let fetch = |_: &str| {
            calls.set(calls.get() + 1);
            Err(anyhow!("boom"))
        };
        let fresh = cache.fetch_missing_with(["https://a".to_owned()], fetch);
        assert_eq!(calls.get(), 1);
        assert_eq!(fresh.get("https://a").unwrap(), &placeholder());

        let fresh = cache.fetch_missing_with(["https://a".to_owned()], fetch);
        assert_eq!(calls.get(), 1, "recent failure is not retried");
        assert!(fresh.is_empty());
    }

    #[test]
    fn failure_is_retried_after_it_ages_past_retry_after() {
        let mut cache = AvatarCache::default();
        let calls = std::cell::Cell::new(0);
        let fail = |_: &str| {
            calls.set(calls.get() + 1);
            Err(anyhow!("boom"))
        };
        let fresh = cache.fetch_missing_with(["https://a".to_owned()], fail);
        assert_eq!(fresh.get("https://a").unwrap(), &placeholder());

        cache.age_failures(RETRY_AFTER + Duration::from_secs(1));

        let succeed = |_: &str| Ok(solid([9, 9, 9]));
        let fresh = cache.fetch_missing_with(["https://a".to_owned()], succeed);
        assert_eq!(fresh.get("https://a").unwrap(), &solid([9, 9, 9]));
    }
}
