//! Vertical scroll stitching.
//!
//! The input is a sequence of frames captured while a page scrolled upwards; the
//! output is one tall image. Everything here is a pure function over pixels: no
//! clocks, no screen access, no files. That makes the hardest part of the product
//! testable with synthetic frames (`tests/stitch.rs`).
//!
//! Pipeline per consecutive frame pair:
//!
//! 1. **Fixed rows** — sticky headers/footers do not move, so the same `y` holds the
//!    same content. Runs of identical rows at the top/bottom are cut out of matching
//!    and from the composite (the header is kept once, the footer is dropped).
//! 2. **Motion columns** — a fixed sidebar or floating button is identical at the same
//!    `x` in both frames. Such columns carry no displacement information and would
//!    inflate every candidate score, so they are masked out.
//! 3. **Displacement** — zero-mean normalised cross-correlation over the masked
//!    region, for every candidate offset. ZNCC is immune to the brightness drift that
//!    breaks SAD (fade-in animations, translucent overlays, HDR conversion).
//! 4. **Composition** — integer row copies only. Interpolation would blur text, and
//!    the frames already agree pixel for pixel.
//!
//! See `spec/04-scrolling-capture.md` for the production version (coarse-to-fine
//! search, multi-band voting, lazy-load settling, window-level capture).

#![forbid(unsafe_code)]

use std::fmt;

/// One 8-bit grayscale image. Luma, not RGB: correlation does not need colour, and
/// dropping two thirds of the bytes makes the search three times cheaper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gray {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

impl Gray {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            data: vec![0; width * height],
        }
    }

    pub fn from_bytes(width: usize, height: usize, data: Vec<u8>) -> Self {
        assert_eq!(data.len(), width * height, "pixel buffer size mismatch");
        Self {
            width,
            height,
            data,
        }
    }

    #[inline]
    pub fn px(&self, x: usize, y: usize) -> u8 {
        self.data[y * self.width + x]
    }

    #[inline]
    pub fn set(&mut self, x: usize, y: usize, value: u8) {
        self.data[y * self.width + x] = value;
    }

    /// Copies `rows` rows starting at `src_y` in `src` to `dst_y` in `self`.
    pub fn copy_rows(&mut self, dst_y: usize, src: &Gray, src_y: usize, rows: usize) {
        if rows == 0 {
            return;
        }
        let width = self.width.min(src.width);
        for offset in 0..rows {
            let from = (src_y + offset) * src.width;
            let to = (dst_y + offset) * self.width;
            self.data[to..to + width].copy_from_slice(&src.data[from..from + width]);
        }
    }
}

/// Converts packed RGBA8 (non-premultiplied) into luma with BT.601 weights.
pub fn luma_from_rgba(rgba: &[u8], width: usize, height: usize) -> Option<Gray> {
    if rgba.len() < width * height * 4 {
        return None;
    }
    let mut gray = Gray::new(width, height);
    for index in 0..width * height {
        let r = rgba[index * 4] as u32;
        let g = rgba[index * 4 + 1] as u32;
        let b = rgba[index * 4 + 2] as u32;
        // 0.299/0.587/0.114 in 8.8 fixed point.
        gray.data[index] = ((77 * r + 150 * g + 29 * b) >> 8) as u8;
    }
    Some(gray)
}

#[derive(Clone, Debug)]
pub struct StitchOptions {
    /// Minimum vertical overlap between two frames. Below this the estimate is not
    /// trustworthy even if the correlation happens to be high.
    pub min_overlap: usize,
    /// A row whose mean absolute difference is at or below this counts as "did not
    /// move" (sticky header/footer, and sensor noise).
    pub fixed_row_threshold: u8,
    /// A column whose mean absolute difference is at or below this counts as static
    /// (fixed sidebar, floating button).
    pub motion_column_threshold: u8,
    /// Below this fraction of moving columns the frame pair is treated as "no scroll".
    pub min_motion_columns: f32,
    /// Minimum ZNCC to accept an offset.
    pub confidence: f32,
    /// Row subsampling during correlation (1 = every row).
    pub sample_step_y: usize,
    /// Hard cap on the output height.
    pub max_height: usize,
    /// Hard cap on captured frames.
    pub max_frames: usize,
}

impl Default for StitchOptions {
    fn default() -> Self {
        Self {
            min_overlap: 48,
            fixed_row_threshold: 6,
            motion_column_threshold: 4,
            min_motion_columns: 0.15,
            confidence: 0.85,
            sample_step_y: 2,
            max_height: 20_000,
            max_frames: 200,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum StitchError {
    NoFrames,
    SizeMismatch,
    /// Frames share too little content to align; the page scrolled more than the
    /// viewport, so retrying with a smaller step is the only fix.
    NoOverlap {
        frame: usize,
    },
    /// Correlation never reached `confidence`. Returning a composite here would be a
    /// silently wrong image, so the caller must degrade (manual mode / raw frames).
    LowConfidence {
        frame: usize,
        offset: usize,
        score: f32,
    },
}

impl fmt::Display for StitchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoFrames => write!(f, "no frames to stitch"),
            Self::SizeMismatch => write!(f, "frames have different dimensions"),
            Self::NoOverlap { frame } => {
                write!(f, "frame {frame} has no usable overlap with the previous one")
            }
            Self::LowConfidence {
                frame,
                offset,
                score,
            } => write!(
                f,
                "frame {frame}: best offset {offset} scored {score:.3}, below the confidence threshold"
            ),
        }
    }
}

impl std::error::Error for StitchError {}

/// One contiguous copy: `rows` rows starting at `src_y` in `frames[frame]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Band {
    pub frame: usize,
    pub src_y: usize,
    pub rows: usize,
}

#[derive(Clone, Debug)]
pub struct Stitched {
    pub image: Gray,
    /// Real composite height. Stored separately so `stitch_plan` can skip building `image`.
    pub height: usize,
    /// Exact composition: replay this against the original (colour) frames.
    pub plan: Vec<Band>,
    /// Vertical displacement accepted for each consecutive frame pair.
    pub offsets: Vec<usize>,
    /// Height of the sticky header that was kept once at the top.
    pub header_height: usize,
    /// The last pair did not move: the page reached its end.
    pub stalled: bool,
    /// The output hit `max_height` or `max_frames` and is incomplete.
    pub truncated: bool,
}

impl Stitched {
    pub fn height(&self) -> usize {
        self.height
    }
}

/// Stitches a vertical scroll sequence.
///
/// Frames must all have the same dimensions; each frame is a full viewport with the
/// page scrolled by an unknown, monotonically increasing amount.
pub fn stitch(frames: &[Gray], opts: &StitchOptions) -> Result<Stitched, StitchError> {
    stitch_impl(frames, opts, true)
}

/// Same matching/composition plan as `stitch`, but without building the final grey
/// composite. Production re-renders from the original colour frames via `plan`, so the
/// full-size grey image (2880×20000 ≈ 57 MB) is never read — skip its allocation/copy.
pub fn stitch_plan(frames: &[Gray], opts: &StitchOptions) -> Result<Stitched, StitchError> {
    stitch_impl(frames, opts, false)
}

fn stitch_impl(
    frames: &[Gray],
    opts: &StitchOptions,
    compose: bool,
) -> Result<Stitched, StitchError> {
    let first = frames.first().ok_or(StitchError::NoFrames)?;
    if frames
        .iter()
        .any(|frame| frame.width != first.width || frame.height != first.height)
    {
        return Err(StitchError::SizeMismatch);
    }
    if first.width == 0 || first.height == 0 {
        return Err(StitchError::SizeMismatch);
    }
    if opts.min_overlap == 0 {
        return Err(StitchError::NoOverlap { frame: 0 });
    }

    let height = first.height;
    let mut offsets = Vec::new();
    let mut stalled = false;
    let mut truncated = false;

    // Pass 1: find the fixed (sticky) band of each pair. Adjacent pairs see the same
    // sticky header/footer, but blank page rows next to it make the detected height
    // wobble by a few rows. Using one pair's height for another pair's band was what
    // duplicated/skipped rows at a seam; the whole capture instead uses a single
    // consensus region (pass 2) so every band ends on the same page row.
    struct Pair {
        columns: Vec<usize>,
        top: usize,
        bottom: usize,
    }
    let mut pairs: Vec<Pair> = Vec::new();
    for index in 1..frames.len() {
        if pairs.len() >= opts.max_frames {
            truncated = true;
            break;
        }
        let prev = &frames[index - 1];
        let next = &frames[index];

        // Columns that move at all: a fixed sidebar is excluded from both the
        // "what is a fixed row" decision and the correlation.
        let columns = motion_columns(prev, next, 0, height, opts);
        if (columns.len() as f32) < opts.min_motion_columns * first.width as f32 {
            stalled = true;
            break;
        }

        let (top, bottom) = detect_fixed_rows(prev, next, &columns, opts);
        if height.saturating_sub(top + bottom) < opts.min_overlap * 2 {
            // The whole frame looked "fixed": the page is not moving any more.
            stalled = true;
            break;
        }
        pairs.push(Pair {
            columns,
            top,
            bottom,
        });
    }

    let (header, footer) = if pairs.is_empty() {
        (0, 0)
    } else {
        let mut tops: Vec<usize> = pairs.iter().map(|pair| pair.top).collect();
        let mut bottoms: Vec<usize> = pairs.iter().map(|pair| pair.bottom).collect();
        tops.sort_unstable();
        bottoms.sort_unstable();
        let top = tops[tops.len() / 2];
        let bottom = bottoms[bottoms.len() / 2];
        // The fixed bands can never eat the whole frame.
        let limit = height.saturating_sub(opts.min_overlap * 2);
        let top = top.min(limit);
        (top, bottom.min(limit - top))
    };
    let body = height.saturating_sub(header + footer);
    // A region that leaves no room to match cannot be right (one pair detected a
    // half-page as sticky); fall back to the full frame.
    let (header, body) = if body < opts.min_overlap * 2 {
        (0, height)
    } else {
        (header, body)
    };

    // Pass 2: match every pair inside that one region.
    for (index, pair) in pairs.iter().enumerate() {
        let prev = &frames[index];
        let next = &frames[index + 1];
        // Offset 0 is a candidate on purpose: it is how "you are already at the
        // bottom" is detected, without a separate near-identical-frames heuristic
        // that would break as soon as the capture has noise.
        let (best_offset, best_score) = search(prev, next, header, body, &pair.columns, opts);

        // Check confidence FIRST. With constant/blank content every offset scores 0.0,
        // so offset 0 wins; treating that as "reached the bottom" silently returned a
        // one-screen image. A real bottom must still be a confident match.
        if best_score < opts.confidence {
            return Err(StitchError::LowConfidence {
                frame: index + 1,
                offset: best_offset,
                score: best_score,
            });
        }
        if best_offset == 0 {
            stalled = true;
            break;
        }
        offsets.push(best_offset);
    }

    let total: usize = header + body + offsets.iter().sum::<usize>();
    let out_height = total.min(opts.max_height);
    if total > out_height {
        truncated = true;
    }

    // The plan is the stitcher's real output: which rows of which frame end up where.
    // The grayscale image is just one rendering of it; callers that need colour (the
    // app, which captures RGBA) replay the same plan without duplicating the logic.
    let mut plan = Vec::with_capacity(offsets.len() + 2);
    let mut budget = out_height;
    if header > 0 {
        push_band(&mut plan, &mut budget, 0, 0, header);
    }
    push_band(&mut plan, &mut budget, 0, header, body);
    for (pair, &offset) in offsets.iter().enumerate() {
        if budget == 0 {
            truncated = true;
            break;
        }
        push_band(
            &mut plan,
            &mut budget,
            pair + 1,
            header + body - offset,
            offset,
        );
    }

    let image = if compose {
        let mut image = Gray::new(first.width, out_height);
        let mut cursor = 0;
        for band in &plan {
            image.copy_rows(cursor, &frames[band.frame], band.src_y, band.rows);
            cursor += band.rows;
        }
        image
    } else {
        // `height` is stored separately; plan-only callers pay no composite allocation.
        Gray::new(1, 1)
    };

    Ok(Stitched {
        image,
        height: out_height,
        plan,
        offsets,
        header_height: header,
        stalled,
        truncated,
    })
}

/// Appends a band, clipped to the remaining height budget.
fn push_band(plan: &mut Vec<Band>, budget: &mut usize, frame: usize, src_y: usize, rows: usize) {
    let rows = rows.min(*budget);
    if rows > 0 {
        plan.push(Band { frame, src_y, rows });
        *budget -= rows;
    }
}

/// Runs of rows at the top and bottom that did not move between two frames.
///
/// Bounded by a third of the frame each: a whole-frame match means "no scroll", which
/// is handled by the displacement search instead.
///
/// A run of identical rows is only a fixed header/footer if it actually *has content*.
/// Blank rows between two paragraphs are identical in both frames (white on white) but
/// they are page content, not something pinned to the viewport. Trimming them used to
/// shift the frame's source row by a few pixels, which is exactly the "one line is
/// duplicated at the seam" artefact: a run of pure background is left alone.
fn detect_fixed_rows(
    prev: &Gray,
    next: &Gray,
    columns: &[usize],
    opts: &StitchOptions,
) -> (usize, usize) {
    let height = prev.height;
    let limit = height / 3;
    let threshold = opts.fixed_row_threshold as f32;
    // A sticky header is *pixel identical* between frames; content that merely has low
    // vertical contrast can sit just under the loose threshold. Trimming back to the
    // "almost exactly equal" prefix is what separates the two — without it, a smooth
    // page can be mistaken for one giant header and the whole frame is discarded.
    let strict = threshold * 0.6;
    // Ink against background; a uniform row is page whitespace, not a pinned element.
    let content = opts.fixed_row_threshold * 2;

    let mut top = 0;
    let mut top_content = 0;
    while top < limit && row_difference(prev, next, top) <= threshold {
        if row_has_content(prev, top, columns, content) {
            top_content += 1;
        }
        top += 1;
    }
    if top_content == 0 {
        top = 0;
    }
    while top > 0 && row_difference(prev, next, top - 1) > strict {
        top -= 1;
    }

    let mut bottom = 0;
    let mut bottom_content = 0;
    while bottom < limit && row_difference(prev, next, height - 1 - bottom) <= threshold {
        if row_has_content(prev, height - 1 - bottom, columns, content) {
            bottom_content += 1;
        }
        bottom += 1;
    }
    if bottom_content == 0 {
        bottom = 0;
    }
    while bottom > 0 && row_difference(prev, next, height - bottom) > strict {
        bottom -= 1;
    }
    (top, bottom)
}

/// Whether a row carries ink (rather than being a uniform background band) over the
/// columns that actually move.
fn row_has_content(image: &Gray, y: usize, columns: &[usize], threshold: u8) -> bool {
    if columns.is_empty() || y >= image.height {
        return false;
    }
    let mut low = u8::MAX;
    let mut high = 0u8;
    for &x in columns {
        if x >= image.width {
            continue;
        }
        let value = image.px(x, y);
        low = low.min(value);
        high = high.max(value);
    }
    high.saturating_sub(low) > threshold
}

fn row_difference(prev: &Gray, next: &Gray, y: usize) -> f32 {
    let width = prev.width.min(next.width);
    if width == 0 {
        return 0.0;
    }
    let start = y * prev.width;
    let mut sum = 0u64;
    for x in 0..width {
        let a = prev.data[start + x] as i32;
        let b = next.data[y * next.width + x] as i32;
        sum += (a - b).unsigned_abs() as u64;
    }
    sum as f32 / width as f32
}

/// Columns that actually move between the two frames.
///
/// A fixed sidebar is identical at the same `x` in both frames regardless of the
/// scroll offset, so `mean |a - b|` over the content rows is ~0. Keeping it in the
/// correlation would give every candidate offset a similar baseline and let a wrong
/// offset win.
fn motion_columns(
    prev: &Gray,
    next: &Gray,
    top: usize,
    body: usize,
    opts: &StitchOptions,
) -> Vec<usize> {
    let width = prev.width.min(next.width);
    let mut columns = Vec::with_capacity(width);
    let threshold = opts.motion_column_threshold as f32;
    for x in 0..width {
        let mut sum = 0u64;
        for row in 0..body {
            let a = prev.px(x, top + row) as i32;
            let b = next.px(x, top + row) as i32;
            sum += (a - b).unsigned_abs() as u64;
        }
        if sum as f32 / body as f32 > threshold {
            columns.push(x);
        }
    }
    columns
}

/// Coarse-to-fine displacement search over an image pyramid.
///
/// A full-resolution scan is `O(offsets × rows × columns)`: on a 1440×900 viewport
/// that is ~64 ms per frame pair, and a 10-screen page would take over a second. Each
/// halving of both dimensions cuts the work 8×, so the search runs on a pyramid
/// (down to ~64 rows tall) and refines ±`factor` at every level. The exact optimum is
/// always within one cell of the coarser answer, so the last level still returns a
/// pixel-accurate offset.
fn search(
    prev: &Gray,
    next: &Gray,
    top: usize,
    body: usize,
    columns: &[usize],
    opts: &StitchOptions,
) -> (usize, f32) {
    let levels = pyramid(body);

    let mut center: Option<usize> = None;
    let mut radius = 0usize;
    let mut best = (0usize, f32::NEG_INFINITY);

    for &factor in &levels {
        // Owned downsampled pair; kept alive for the whole iteration so the rest of
        // the loop can work with plain references.
        let owned = if factor == 1 {
            None
        } else {
            Some((downsample(prev, factor), downsample(next, factor)))
        };
        let (image_a, image_b) = match &owned {
            Some((a, b)) => (a, b),
            None => (prev, next),
        };

        let level_top = if factor == 1 { top } else { top / factor };
        let level_body = if factor == 1 {
            body
        } else {
            (body / factor).min(image_a.height.saturating_sub(level_top))
        };
        let level_overlap = if factor == 1 {
            opts.min_overlap
        } else {
            (opts.min_overlap / factor).max(2)
        };
        if level_body < level_overlap * 2 {
            continue;
        }

        let mut level_columns = if factor == 1 {
            columns.to_vec()
        } else {
            motion_columns(image_a, image_b, level_top, level_body, opts)
        };
        if level_columns.len() < 2 {
            continue;
        }
        if factor > 1 {
            // Statistics do not need every column; capping the coarse pass at ~128
            // keeps it proportional to the viewport rather than to its width.
            let stride = level_columns.len().div_ceil(128);
            if stride > 1 {
                level_columns = level_columns.into_iter().step_by(stride).collect();
            }
        }

        let max_offset = level_body.saturating_sub(level_overlap);
        let (low, high) = match center {
            None => (0, max_offset),
            Some(center) => {
                let low = center.saturating_sub(radius) / factor;
                let high = ((center + radius) / factor + 1).min(max_offset);
                (low.min(max_offset), high)
            }
        };
        if low > high {
            continue;
        }

        let step_y = if factor == 1 { opts.sample_step_y } else { 1 };
        let mut local = (low, f32::NEG_INFINITY);
        for offset in low..=high {
            let score = correlation(
                image_a,
                image_b,
                level_top,
                offset,
                level_body - offset,
                &level_columns,
                step_y,
            );
            if score > local.1 {
                local = (offset, score);
            }
        }
        if local.1 == f32::NEG_INFINITY {
            continue;
        }
        center = Some(local.0 * factor);
        radius = factor;
        best = (local.0 * factor, local.1);
    }

    // The coarse levels can lock onto noise: box-filtering 1px glyph strokes makes the
    // image nearly uniform, and the narrow refinement window then cannot recover. If the
    // fast answer is not confident enough, scan every offset at full resolution — it
    // costs ~50 ms but only happens for a pair the fast path would otherwise reject.
    if best.1 < opts.confidence {
        // Statistics do not need every column; keeping the fallback under ~256 columns
        // holds it to a few tens of milliseconds on a Retina frame.
        let mut sampled = columns.to_vec();
        let stride = sampled.len().div_ceil(256);
        if stride > 1 {
            sampled = sampled.into_iter().step_by(stride).collect();
        }
        let max_offset = body.saturating_sub(opts.min_overlap);
        let mut full = (0usize, f32::NEG_INFINITY);
        for offset in 0..=max_offset {
            let score = correlation(
                prev,
                next,
                top,
                offset,
                body - offset,
                &sampled,
                opts.sample_step_y,
            );
            if score > full.1 {
                full = (offset, score);
            }
            // A near-perfect Pearson score cannot be beaten; stop scanning the rest of
            // the offsets. This is the common case for repeated/periodic content, where
            // without it every one of the ~body²/2 pixel comparisons is done for nothing.
            if score >= 0.9999 {
                break;
            }
        }
        if full.1 > best.1 {
            best = full;
        }
    }

    best
}

/// Pyramid factors from coarsest to finest.
///
/// Capped at 4×: a strong box filter turns fine texture (text glyphs, 1 px borders)
/// into near-uniform grey, so an 8× level loses the very signal the correlation needs.
/// Four is the sweet spot — still an 8× cost reduction, still recognisable content.
/// The coarsest level also keeps at least 64 content rows, below which the window is
/// too small to discriminate between candidates.
fn pyramid(body: usize) -> Vec<usize> {
    let mut coarsest = 1;
    while coarsest * 2 <= 4 && body / (coarsest * 2) >= 64 {
        coarsest *= 2;
    }
    let mut levels = Vec::new();
    let mut factor = coarsest;
    loop {
        levels.push(factor);
        if factor == 1 {
            break;
        }
        factor /= 2;
    }
    levels
}

/// Box-filtered downsample by `factor`. Averaging (rather than sampling) suppresses
/// one-pixel animation and sensor noise before the coarse pass.
fn downsample(src: &Gray, factor: usize) -> Gray {
    let width = src.width / factor;
    let height = src.height / factor;
    let mut out = Gray::new(width, height);
    let area = (factor * factor) as u32;
    for y in 0..height {
        for x in 0..width {
            let mut sum = 0u32;
            for dy in 0..factor {
                for dx in 0..factor {
                    sum += src.px(x * factor + dx, y * factor + dy) as u32;
                }
            }
            out.set(x, y, (sum / area) as u8);
        }
    }
    out
}

/// Zero-mean normalised cross-correlation between `prev[top + offset ..]` and
/// `next[top ..]` over `overlap` rows, restricted to `columns`.
///
/// Returns a value in `[-1, 1]`; `0` when either side is constant (no information).
fn correlation(
    prev: &Gray,
    next: &Gray,
    top: usize,
    offset: usize,
    overlap: usize,
    columns: &[usize],
    step_y: usize,
) -> f32 {
    let step_y = step_y.max(1);
    let mut count = 0f64;
    let mut sum_a = 0f64;
    let mut sum_b = 0f64;
    let mut sum_aa = 0f64;
    let mut sum_bb = 0f64;
    let mut sum_ab = 0f64;

    let mut row = 0;
    while row < overlap {
        let y_prev = top + offset + row;
        let y_next = top + row;
        for &x in columns {
            let a = prev.px(x, y_prev) as f64;
            let b = next.px(x, y_next) as f64;
            count += 1.0;
            sum_a += a;
            sum_b += b;
            sum_aa += a * a;
            sum_bb += b * b;
            sum_ab += a * b;
        }
        row += step_y;
    }

    let variance_a = count * sum_aa - sum_a * sum_a;
    let variance_b = count * sum_bb - sum_b * sum_b;
    let denominator = (variance_a * variance_b).sqrt();
    if denominator <= 1e-9 {
        return 0.0;
    }
    ((count * sum_ab - sum_a * sum_b) / denominator) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_rgba_to_luma() {
        let rgba = [255u8, 255, 255, 255, 0, 0, 0, 255];
        let gray = luma_from_rgba(&rgba, 2, 1).unwrap();
        assert!(gray.px(0, 0) > 250);
        assert_eq!(gray.px(1, 0), 0);
        assert!(luma_from_rgba(&rgba, 3, 1).is_none());
    }

    #[test]
    fn copy_rows_moves_a_band() {
        let mut dst = Gray::new(4, 4);
        let mut src = Gray::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                src.set(x, y, (y * 10) as u8);
            }
        }
        dst.copy_rows(1, &src, 2, 2);
        assert_eq!(dst.px(0, 1), 20);
        assert_eq!(dst.px(3, 2), 30);
    }
}
