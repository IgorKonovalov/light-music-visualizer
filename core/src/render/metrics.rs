//! Pure image metrics over [`CaptureImage`]s (Plan 0013): pixel and shape
//! difference plus coverage/spread, shared by the differential visual-QA tests
//! and the `shot` CLI report.
//!
//! Everything here is a pure function of its input pixels — no GPU, no clock, no
//! allocation beyond the small working buffers. Not a per-frame hot path, but it
//! lives under `render/` so it carries the panic-denial pragma (and the hygiene
//! guard needs it): written index- and panic-free throughout.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

use super::CaptureImage;

/// Grid the shape metric downscales to before edge detection (~32×32).
const STRUCT_GRID: usize = 32;

/// Mean absolute per-channel (RGB) difference between two images, normalized to
/// `0.0..=1.0` (0 = identical, 1 = every channel maximally different). Mismatched
/// dimensions read as fully different (`1.0`). Alpha is ignored — the capture
/// background is opaque, so alpha carries no signal.
pub fn frame_diff(a: &CaptureImage, b: &CaptureImage) -> f32 {
    if a.width != b.width || a.height != b.height || a.rgba.len() != b.rgba.len() {
        return 1.0;
    }
    let mut sum: u64 = 0;
    let mut count: u64 = 0;
    for (pa, pb) in a.rgba.chunks_exact(4).zip(b.rgba.chunks_exact(4)) {
        for c in 0..3 {
            if let (Some(&x), Some(&y)) = (pa.get(c), pb.get(c)) {
                sum += x.abs_diff(y) as u64;
                count += 1;
            }
        }
    }
    if count == 0 {
        return 0.0;
    }
    sum as f32 / (count as f32 * 255.0)
}

/// Shape-aware difference in `0.0..=1.0`: downscale each image to a small
/// grayscale grid, take the Sobel edge magnitude, normalize each edge map by its
/// own peak, and mean-abs-diff them. Normalizing per-image cancels overall
/// contrast, so a **recolor of the same shape** scores low while a **different
/// shape** scores high — the near-duplicate probe (an approximation of SSIM).
pub fn struct_diff(a: &CaptureImage, b: &CaptureImage) -> f32 {
    let ea = normalize_max(&sobel(&downscale_gray(a)));
    let eb = normalize_max(&sobel(&downscale_gray(b)));
    let mut sum = 0.0f32;
    let mut count = 0.0f32;
    for (x, y) in ea.iter().zip(eb.iter()) {
        sum += (x - y).abs();
        count += 1.0;
    }
    if count == 0.0 {
        return 0.0;
    }
    (sum / count).clamp(0.0, 1.0)
}

/// Fraction of pixels whose RGB differs from `bg` by more than `eps` on any
/// channel — "how much of the frame is lit" (`0.0..=1.0`). Alpha is ignored.
pub fn coverage(img: &CaptureImage, bg: [u8; 4], eps: u8) -> f32 {
    let mut lit: u64 = 0;
    let mut total: u64 = 0;
    for px in img.rgba.chunks_exact(4) {
        total += 1;
        if is_lit(px, bg, eps) {
            lit += 1;
        }
    }
    if total == 0 {
        return 0.0;
    }
    lit as f32 / total as f32
}

/// How many of the four image quadrants contain at least one lit pixel
/// (`0..=4`) — a cheap "not just a dot in one corner" spread check.
pub fn quadrant_spread(img: &CaptureImage, bg: [u8; 4], eps: u8) -> u8 {
    let w = img.width as usize;
    let h = img.height as usize;
    if w == 0 || h == 0 {
        return 0;
    }
    let mut hit = [false; 4];
    for (i, px) in img.rgba.chunks_exact(4).enumerate() {
        if !is_lit(px, bg, eps) {
            continue;
        }
        let x = i % w;
        let y = i / w;
        let qx = usize::from(x >= w / 2);
        let qy = usize::from(y >= h / 2);
        if let Some(slot) = hit.get_mut(qy * 2 + qx) {
            *slot = true;
        }
    }
    hit.iter().filter(|&&b| b).count() as u8
}

/// Whether a pixel's RGB differs from `bg` by more than `eps` on any channel.
fn is_lit(px: &[u8], bg: [u8; 4], eps: u8) -> bool {
    px.iter()
        .zip(bg.iter())
        .take(3)
        .any(|(&c, &b)| c.abs_diff(b) > eps)
}

/// Box-average an image down to a `STRUCT_GRID`×`STRUCT_GRID` grid of grayscale
/// luma in `0.0..=1.0`.
fn downscale_gray(img: &CaptureImage) -> Vec<f32> {
    let g = STRUCT_GRID;
    let mut cells = vec![0.0f32; g * g];
    let mut counts = vec![0u32; g * g];
    let w = img.width as usize;
    let h = img.height as usize;
    if w == 0 || h == 0 {
        return cells;
    }
    for (i, px) in img.rgba.chunks_exact(4).enumerate() {
        let x = i % w;
        let y = i / w;
        let cx = (x * g / w).min(g - 1);
        let cy = (y * g / h).min(g - 1);
        let idx = cy * g + cx;
        let luma = 0.299 * px.first().copied().unwrap_or(0) as f32
            + 0.587 * px.get(1).copied().unwrap_or(0) as f32
            + 0.114 * px.get(2).copied().unwrap_or(0) as f32;
        if let (Some(cell), Some(cnt)) = (cells.get_mut(idx), counts.get_mut(idx)) {
            *cell += luma;
            *cnt += 1;
        }
    }
    for (cell, cnt) in cells.iter_mut().zip(counts.iter()) {
        if *cnt > 0 {
            *cell /= *cnt as f32 * 255.0;
        }
    }
    cells
}

/// Sobel gradient magnitude over a `STRUCT_GRID`×`STRUCT_GRID` grayscale grid.
/// Border cells stay zero (no wrap).
fn sobel(gray: &[f32]) -> Vec<f32> {
    let g = STRUCT_GRID;
    let mut edges = vec![0.0f32; g * g];
    let at = |x: usize, y: usize| -> f32 { gray.get(y * g + x).copied().unwrap_or(0.0) };
    for y in 1..g.saturating_sub(1) {
        for x in 1..g.saturating_sub(1) {
            let gx = at(x + 1, y - 1) + 2.0 * at(x + 1, y) + at(x + 1, y + 1)
                - at(x - 1, y - 1)
                - 2.0 * at(x - 1, y)
                - at(x - 1, y + 1);
            let gy = at(x - 1, y + 1) + 2.0 * at(x, y + 1) + at(x + 1, y + 1)
                - at(x - 1, y - 1)
                - 2.0 * at(x, y - 1)
                - at(x + 1, y - 1);
            if let Some(e) = edges.get_mut(y * g + x) {
                *e = (gx * gx + gy * gy).sqrt();
            }
        }
    }
    edges
}

/// Scale a map so its peak is 1.0; an all-zero map is returned unchanged.
fn normalize_max(v: &[f32]) -> Vec<f32> {
    let max = v.iter().copied().fold(0.0f32, f32::max);
    if max <= f32::EPSILON {
        return v.to_vec();
    }
    v.iter().map(|x| x / max).collect()
}

#[cfg(test)]
mod tests {
    // Test bodies index and unwrap freely — not the hot path.
    #![allow(clippy::indexing_slicing, clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    const BLACK: [u8; 4] = [0, 0, 0, 255];

    /// Build a `w`×`h` image by painting each pixel from `f(x, y) -> [r,g,b,a]`.
    fn image(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> CaptureImage {
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                rgba.extend_from_slice(&f(x, y));
            }
        }
        CaptureImage {
            width: w,
            height: h,
            rgba,
        }
    }

    fn solid(w: u32, h: u32, color: [u8; 4]) -> CaptureImage {
        image(w, h, |_, _| color)
    }

    /// A vertical bar in `color` over black covering `x < w/2`.
    fn left_bar(w: u32, h: u32, color: [u8; 4]) -> CaptureImage {
        image(w, h, |x, _| if x < w / 2 { color } else { BLACK })
    }

    /// A horizontal bar in `color` over black covering `y < h/2`.
    fn top_bar(w: u32, h: u32, color: [u8; 4]) -> CaptureImage {
        image(w, h, |_, y| if y < h / 2 { color } else { BLACK })
    }

    #[test]
    fn frame_diff_bounds() {
        let black = solid(32, 32, BLACK);
        let white = solid(32, 32, [255, 255, 255, 255]);
        assert_eq!(frame_diff(&black, &black), 0.0);
        assert_eq!(frame_diff(&black, &white), 1.0);
        // Mismatched sizes read as fully different.
        assert_eq!(frame_diff(&black, &solid(16, 16, BLACK)), 1.0);
    }

    #[test]
    fn coverage_and_spread_extremes() {
        let black = solid(32, 32, BLACK);
        let white = solid(32, 32, [255, 255, 255, 255]);
        assert_eq!(coverage(&black, BLACK, 8), 0.0);
        assert_eq!(coverage(&white, BLACK, 8), 1.0);
        assert_eq!(quadrant_spread(&black, BLACK, 8), 0);
        assert_eq!(quadrant_spread(&white, BLACK, 8), 4);

        // A single lit pixel in the top-left quadrant hits exactly one quadrant.
        let dot = image(32, 32, |x, y| {
            if x == 2 && y == 2 {
                [255, 255, 255, 255]
            } else {
                BLACK
            }
        });
        assert_eq!(quadrant_spread(&dot, BLACK, 8), 1);
    }

    #[test]
    fn struct_diff_is_recolor_robust_but_shape_sensitive() {
        // Same shape (left bar), different colors: low structural difference.
        let red_bar = left_bar(64, 64, [220, 30, 30, 255]);
        let blue_bar = left_bar(64, 64, [30, 30, 220, 255]);
        let recolor = struct_diff(&red_bar, &blue_bar);

        // Different shape (left bar vs top bar), same color: high difference.
        let red_top = top_bar(64, 64, [220, 30, 30, 255]);
        let reshape = struct_diff(&red_bar, &red_top);

        assert!(
            recolor < reshape,
            "a recolor ({recolor:.3}) must read as more similar than a reshape ({reshape:.3})"
        );
        assert!(
            recolor < 0.05,
            "same-shape recolor is near-zero ({recolor:.3})"
        );
        assert!(
            reshape > 0.10,
            "a genuine shape change is clearly nonzero ({reshape:.3})"
        );
    }
}
