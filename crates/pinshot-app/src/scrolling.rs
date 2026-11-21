//! Colour scroll stitching for the app.
//!
//! `pinshot-stitch` matches on luminance and produces a `plan` — the exact list of
//! `(frame, source row, count)` copies. This module replays that plan over the
//! original RGBA frames, so the long image keeps its colours and the matching logic
//! stays in one place.

use pinshot_core::{Error, Image, Rect, Result};
use pinshot_stitch::{luma_from_rgba, stitch_plan, StitchError, StitchOptions};

/// Stitches while dropping frames that cannot be matched.
///
/// A fast scroll can move more than a viewport between two captures; that pair has no
/// overlap at all, and failing the whole capture because of it is not helpful. Such a
/// frame is skipped (the slice it covered is lost) and the rest is stitched normally.
pub fn stitch_images_tolerant(frames: &[Image], options: &StitchOptions) -> Result<Image> {
    if frames.len() < 2 {
        return stitch_images(frames, options);
    }
    let mut kept: Vec<&Image> = vec![&frames[0]];
    let (width, height) = (frames[0].width as usize, frames[0].height as usize);
    // Reuse the anchor's luma across pairs: re-converting it for every frame duplicated
    // an RGBA→luma pass, especially when several frames were skipped. Each frame is now
    // converted once and the surviving one becomes the next anchor.
    let mut anchor_luma = luma_from_rgba(&frames[0].rgba, width, height);
    for frame in frames.iter().skip(1) {
        let Some(a) = anchor_luma.clone() else {
            continue;
        };
        let Some(b) = luma_from_rgba(&frame.rgba, width, height) else {
            continue;
        };
        if let Ok(stitched) = stitch_plan(&[a, b.clone()], options) {
            if !stitched.offsets.is_empty() {
                kept.push(frame);
                anchor_luma = Some(b);
            }
        }
    }
    if kept.len() < 2 {
        return Err(Error::internal(
            "滚动拼接失败：没有两帧能对上（滚动太快，帧之间没有重叠）",
        ));
    }
    let owned: Vec<Image> = kept.into_iter().cloned().collect();
    stitch_images(&owned, options)
}

pub fn stitch_images(frames: &[Image], options: &StitchOptions) -> Result<Image> {
    if frames.is_empty() {
        return Err(Error::internal("no frames to stitch"));
    }
    let (width, height) = (frames[0].width as usize, frames[0].height as usize);
    if frames
        .iter()
        .any(|frame| frame.width as usize != width || frame.height as usize != height)
    {
        return Err(Error::internal("frames have different dimensions"));
    }

    let gray: Vec<pinshot_stitch::Gray> = frames
        .iter()
        .map(|frame| {
            luma_from_rgba(&frame.rgba, width, height)
                .ok_or_else(|| Error::internal("frame buffer does not match its size"))
        })
        .collect::<Result<Vec<_>>>()?;

    let stitched = stitch_plan(&gray, options).map_err(|error| map_error(error, frames.len()))?;

    let out_height = stitched.height() as u32;
    let mut output = Image::new(width as u32, out_height);
    let row_bytes = width * 4;
    let mut cursor = 0u32;
    for band in &stitched.plan {
        let source = &frames[band.frame];
        for row in 0..band.rows {
            let src = ((band.src_y + row) * width) * 4;
            let dst = ((cursor as usize + row) * width) * 4;
            output.rgba[dst..dst + row_bytes].copy_from_slice(&source.rgba[src..src + row_bytes]);
        }
        cursor += band.rows as u32;
    }
    Ok(output)
}

/// Collects frames while the user scrolls, applying the "settle" rules from
/// `spec/04-scrolling-capture.md` §6: a frame is only accepted once the picture has
/// stopped moving, and two identical frames in a row mean the page has ended.
pub struct ScrollCollector {
    frames: Vec<Image>,
    options: StitchOptions,
    max_frames: usize,
    duplicates: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushOutcome {
    Accepted,
    /// The frame was (almost) identical to the previous one; not added.
    Duplicate,
    Stopped(StopReason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// Two identical frames in a row: nothing left to scroll.
    EndOfContent,
    FrameLimit,
}

impl ScrollCollector {
    pub fn new(options: StitchOptions, max_frames: usize) -> Self {
        Self {
            frames: Vec::new(),
            options,
            max_frames,
            duplicates: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn frames(&self) -> &[Image] {
        &self.frames
    }

    pub fn push(&mut self, frame: Image) -> PushOutcome {
        if let Some(previous) = self.frames.last() {
            if nearly_identical(previous, &frame) {
                self.duplicates += 1;
                if self.duplicates >= 2 {
                    return PushOutcome::Stopped(StopReason::EndOfContent);
                }
                return PushOutcome::Duplicate;
            }
            self.duplicates = 0;
        }
        if self.frames.len() >= self.max_frames {
            return PushOutcome::Stopped(StopReason::FrameLimit);
        }
        self.frames.push(frame);
        PushOutcome::Accepted
    }

    pub fn finish(&self) -> Result<Image> {
        stitch_images(&self.frames, &self.options)
    }

    /// Takes the collected frames, for callers that need to reorder them (a manual
    /// capture can run in either scroll direction).
    pub fn into_frames(self) -> Vec<Image> {
        self.frames
    }
}

/// Knobs for one auto-scroll run (`spec/04-scrolling-capture.md` §5).
/// Which way the page moves while frames are collected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Page content moves up (the usual "read further down" scroll).
    Down,
    /// Page content moves down; frames are reversed before stitching.
    Up,
}

#[derive(Clone, Copy, Debug)]
pub struct ScrollRunOptions {
    /// `Down` for the common case, `Up` when the user started at the bottom edge.
    pub direction: Direction,
    /// How far to scroll per step, as a fraction of the viewport height.
    pub step_ratio: f32,
    /// Frames to keep at most.
    pub max_frames: usize,
    /// Hard cap on the stitched height, in pixels.
    pub max_height: usize,
}

impl Default for ScrollRunOptions {
    fn default() -> Self {
        Self {
            direction: Direction::Down,
            step_ratio: 0.7,
            max_frames: 40,
            max_height: 20_000,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScrollProgress {
    pub frames: usize,
    pub width: u32,
    pub height: u32,
    /// Frames that had to be dropped because the page moved further than a viewport
    /// between two captures (i.e. the user scrolled too fast).
    pub skipped: usize,
}

/// Drives capture → scroll → capture until the page stops moving.
///
/// The three side effects are injected, so the whole loop is testable without a
/// screen: the test hands in a synthetic page, a scroll counter and a no-op sleep.
/// `scroll` receives the step **in pixels** (the viewport is measured in pixels);
/// the caller converts to whatever the OS wants.
pub fn auto_scroll<C, S, W, P>(
    mut capture: C,
    mut scroll: S,
    mut wait: W,
    options: ScrollRunOptions,
    mut progress: P,
) -> Result<Image>
where
    C: FnMut() -> Result<Image>,
    S: FnMut(u32) -> Result<()>,
    W: FnMut(),
    P: FnMut(ScrollProgress),
{
    let stitch_options = StitchOptions {
        max_height: options.max_height,
        ..Default::default()
    };

    let first = capture()?;
    let viewport = first.height;
    // Upward capture collects frames bottom-to-top, so they are reversed before the
    // stitcher sees them; Down uses `collector` directly. Keeping a second full copy of
    // every frame (≈20 MB each, 60 frames ≈1.2 GB) in the common Down case was pure waste.
    let mut collected: Vec<Image> = Vec::new();
    let upward = options.direction == Direction::Up;
    let min_overlap = stitch_options.min_overlap as u32;
    let mut collector = ScrollCollector::new(stitch_options, options.max_frames);
    // How far one *unit* of the scroll closure moves the page is unknown: a wheel
    // "line" can be 60 px in one app and 200 px in another. Start with one unit,
    // measure the real displacement between frames, then solve for the step that
    // lands at `step_ratio` of the viewport while keeping `min_overlap` rows shared.
    let target = (viewport as f32 * options.step_ratio.clamp(0.2, 0.6)) as u32;
    let max_step = viewport.saturating_sub(min_overlap * 2).max(1);
    // Start at one wheel unit (40 px, i.e. one line in the injector). Starting larger
    // risks tearing the overlap before the first measurement exists, and starting at
    // 1 px makes the measurement indistinguishable from noise.
    let mut step = 40u32.clamp(1, max_step);

    if upward {
        collected.push(first.clone());
    }
    if let PushOutcome::Stopped(_) = collector.push(first) {
        return collector.finish();
    }
    let mut estimate = collector.frames()[0].height;
    progress(ScrollProgress {
        frames: 1,
        width: collector.frames()[0].width,
        height: estimate,
        skipped: 0,
    });

    let mut previous: Option<Image> = collector.frames().first().cloned();
    loop {
        if collector.len() >= options.max_frames {
            break;
        }
        scroll(step)?;
        wait();
        let frame = capture()?;
        // Calibrate the step against reality: measure what that request actually moved
        // and solve for the step that lands at `step_ratio` of the viewport.
        if let Some(before) = &previous {
            // The matcher only searches "content moved up"; for an upward capture the
            // pair has to be measured the other way round.
            let pair = if options.direction == Direction::Up {
                measure_shift(&frame, before)
            } else {
                measure_shift(before, &frame)
            };
            if let Some(moved) = pair {
                if moved >= 8 {
                    let per_unit = moved as f32 / step as f32;
                    let wanted = (target as f32 / per_unit).round().max(1.0) as u32;
                    step = wanted.clamp(1, max_step);
                }
            } else if step > 1 {
                // No usable match: that step tore the overlap apart. Back off.
                step = (step / 2).max(1);
            }
        }
        match collector.push(frame.clone()) {
            PushOutcome::Accepted => {
                if upward {
                    previous = Some(frame.clone());
                    collected.push(frame);
                } else {
                    // The frame is already in `collector`; move it into `previous` instead
                    // of cloning a third full RGBA buffer.
                    previous = Some(frame);
                }
            }
            PushOutcome::Duplicate => continue,
            PushOutcome::Stopped(_) => break,
        }
        // Cheap progress estimate: each accepted frame adds `step`, which is what the
        // user sees growing in the preview before the final stitch runs.
        estimate = estimate.saturating_add(step);
        progress(ScrollProgress {
            frames: collector.len(),
            width: collector.frames()[0].width,
            height: estimate,
            skipped: 0,
        });
    }

    if upward {
        collected.reverse();
        let stitch_options = StitchOptions {
            max_height: options.max_height,
            ..Default::default()
        };
        let mut ordered = ScrollCollector::new(stitch_options, options.max_frames);
        for frame in collected {
            ordered.push(frame);
        }
        return ordered.finish();
    }
    collector.finish()
}

/// What one pair of viewports says about how far the page moved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe {
    /// The page moved this many pixels.
    Moved(usize),
    /// The pair matched but nothing moved: the page is at its end.
    Still,
    /// No usable match — the step was probably larger than the viewport.
    Unmatched,
}

#[derive(Clone, Debug)]
pub enum LiveStep {
    /// New rows were captured; `shift` is how far the page moved, in pixels.
    Grown { strip: Image, shift: usize },
    /// The pair matched but nothing moved: the page has reached its end.
    Stalled,
    /// The pair could not be matched — usually a scroll step larger than the viewport.
    /// The caller should shrink its step and try again without capturing.
    Unmatched,
}

/// Grows a long image one viewport at a time, appending only the rows that are new.
///
/// `auto_scroll` needs every frame before it can stitch anything; the live gesture
/// needs the opposite: rows have to appear on screen while the page is still moving.
/// Measuring the shift between consecutive frames and appending the revealed rows
/// gives exactly that, and costs one pair match per step instead of a full restitch.
pub struct LiveStitcher {
    image: Image,
    /// The previous viewport, kept separately: matching the new frame against the
    /// accumulated image would compare two different heights.
    last: Image,
    direction: Direction,
    /// Sticky header height, discovered from the first match. It is kept once at the
    /// top, so upward growth has to insert *below* it.
    header: usize,
    /// Smallest believable displacement; below this the page is treated as still.
    min_shift: usize,
}

impl LiveStitcher {
    pub fn new(first: Image, direction: Direction, min_shift: usize) -> Self {
        Self {
            last: first.clone(),
            image: first,
            direction,
            header: 0,
            min_shift: min_shift.max(1),
        }
    }

    pub fn image(&self) -> &Image {
        &self.image
    }

    pub fn height(&self) -> u32 {
        self.image.height
    }

    pub fn width(&self) -> u32 {
        self.image.width
    }

    /// Measures how far the page moved between the last frame and `frame`, without
    /// touching the accumulated image. `None` means "no usable match" — either the page
    /// did not move or the step tore the overlap.
    pub fn probe(&mut self, frame: &Image) -> Result<Probe> {
        if frame.width != self.image.width || frame.height != self.last.height {
            return Ok(Probe::Unmatched);
        }
        // The matcher always works "content moved up". For an upward capture the pair
        // is fed the other way round, which is the same trick `auto_scroll` uses.
        let (before, after) = match self.direction {
            Direction::Down => (&self.last, frame),
            Direction::Up => (frame, &self.last),
        };
        let width = before.width as usize;
        let height = before.height as usize;
        let Some(a) = luma_from_rgba(&before.rgba, width, height) else {
            return Ok(Probe::Unmatched);
        };
        let Some(b) = luma_from_rgba(&after.rgba, width, height) else {
            return Ok(Probe::Unmatched);
        };
        let Ok(stitched) = stitch_plan(&[a, b], &StitchOptions::default()) else {
            return Ok(Probe::Unmatched);
        };
        match stitched.offsets.first().copied() {
            Some(shift) if shift >= self.min_shift => {
                if self.header == 0 {
                    self.header = stitched.header_height;
                }
                Ok(Probe::Moved(shift))
            }
            _ => Ok(Probe::Still),
        }
    }

    /// Appends the rows a measured step revealed.
    pub fn append(&mut self, frame: &Image, shift: usize) -> Result<Image> {
        // Sticky header: the same rows in every frame, kept once at the top. New
        // content starts right after it, and upward growth must insert below it.
        let top = self.header as u32;
        let strip = match self.direction {
            // New content fills the bottom of the frame.
            Direction::Down => {
                let rows = shift as u32;
                let start = frame.height.saturating_sub(rows);
                frame.crop(Rect::new(0, start as i32, frame.width, rows))
            }
            // New content fills the top of the frame, below the sticky header.
            Direction::Up => frame.crop(Rect::new(0, top as i32, frame.width, shift as u32)),
        };
        if strip.is_empty() {
            return Err(Error::internal("the revealed strip is empty"));
        }

        let mut merged = Image::new(self.image.width, self.height() + strip.height);
        if self.direction == Direction::Down {
            let offset = self.image.rgba.len();
            merged.rgba[..offset].copy_from_slice(&self.image.rgba);
            merged.rgba[offset..].copy_from_slice(&strip.rgba);
        } else {
            // Keep the header first, then the new rows, then everything that was
            // already captured.
            let header_bytes = top as usize * self.image.width as usize * 4;
            merged.rgba[..header_bytes].copy_from_slice(&self.image.rgba[..header_bytes]);
            let after_header = header_bytes + strip.rgba.len();
            merged.rgba[header_bytes..after_header].copy_from_slice(&strip.rgba);
            merged.rgba[after_header..].copy_from_slice(&self.image.rgba[header_bytes..]);
        }
        self.image = merged;
        self.last = frame.clone();
        Ok(strip)
    }

    /// Adds the rows a new viewport revealed.
    ///
    /// `Unmatched` is the interesting one: it means the scroll step was larger than the
    /// viewport (or the page changed), so the caller should take a smaller step instead
    /// of ending the capture.
    pub fn push(&mut self, frame: &Image) -> Result<LiveStep> {
        match self.probe(frame)? {
            Probe::Moved(shift) => match self.append(frame, shift) {
                Ok(strip) => Ok(LiveStep::Grown { strip, shift }),
                Err(_) => Ok(LiveStep::Unmatched),
            },
            Probe::Still => Ok(LiveStep::Stalled),
            Probe::Unmatched => Ok(LiveStep::Unmatched),
        }
    }
}

/// Drives a live scroll: scroll, capture, append — adapting the step to what the page
/// actually did.
///
/// The Accessibility API scrolls by a fraction of the *whole* scrollable height, which
/// can be far more than a viewport; a fixed fraction tears the overlap every time. This
/// loop measures each step and keeps it near `target` of the viewport, halving it when a
/// pair cannot be matched at all.
///
/// `scroll` receives a signed fraction (negative scrolls up). `on_append` gets the new
/// rows and the image height so far, which is what a live preview draws.
// The side effects are injected one by one on purpose: the loop is tested with a
// synthetic page, a fake scrollbar and a no-op sleep, and bundling them into a struct
// would only hide what each call site has to provide.
#[allow(clippy::too_many_arguments)]
pub fn live_scroll<C, S, W, T, P>(
    mut capture: C,
    mut scroll: S,
    mut wait: W,
    mut should_stop: T,
    direction: Direction,
    max_steps: usize,
    mut on_append: P,
) -> Result<Image>
where
    C: FnMut() -> Result<Image>,
    // `S` returns the scrollbar position change this step asked for (0..1), when known.
    S: FnMut(f32) -> Result<ScrollStep>,
    W: FnMut(),
    T: FnMut() -> bool,
    P: FnMut(&Image, u32),
{
    let first = capture()?;
    let viewport = first.height.max(1);
    let mut stitcher = LiveStitcher::new(first.clone(), direction, 4);
    let mut steps = 0usize;
    let mut stalls = 0usize;
    let mut misses = 0usize;
    // Start very small: on a long page 15 % is dozens of screens, and a repetitive page
    // can even "match" a wrong offset. The first accepted step calibrates everything.
    let mut fraction = 0.01f32;
    let sign = if direction == Direction::Up {
        -1.0
    } else {
        1.0
    };
    let target = viewport as f32 * 0.7;
    // Pixels per unit of scrollbar travel, learned from the first accepted step.
    let mut px_per_unit: Option<f32> = None;
    // Trace aid: lets the log say "the picture did not change" without re-deriving it.
    let mut previous_fingerprint: Option<u64> = None;
    let mut frozen_run = 0usize;

    on_append(&first, stitcher.height());
    while steps < max_steps && stalls < 2 && misses < 8 && !should_stop() {
        let delta = {
            match scroll(fraction * sign)? {
                ScrollStep::Moved(delta) => delta,
                // No accessible scrollbar: stop immediately. The caller switches to the
                // hidden-overlay manual capture; keeping the frozen overlay on screen and
                // asking the user to scroll "behind" it is unusable.
                ScrollStep::Unsupported => {
                    if std::env::var_os("PINSHOT_TRACE").is_some() {
                        eprintln!("live: no accessible scrollbar");
                    }
                    break;
                }
            }
        };
        wait();
        let frame = capture()?;
        let viewport_u32 = frame.height;
        let probed = stitcher.probe(&frame)?;
        // Always computed (it is cheap): a frozen capture has to be reported accurately
        // instead of being mistaken for "the page did not scroll".
        let fingerprint = frame_fingerprint(&frame);
        let frozen = previous_fingerprint == Some(fingerprint);
        let scrollbar_moved = delta.map(|value| value.abs() > 1e-4).unwrap_or(false);
        previous_fingerprint = Some(fingerprint);
        if frozen && scrollbar_moved {
            frozen_run += 1;
            if frozen_run >= 2 {
                return Err(Error::internal(
                    "画面没有变化：这个区域可能不可滚动，或页面已经到底",
                ));
            }
        } else {
            frozen_run = 0;
        }
        if std::env::var_os("PINSHOT_TRACE").is_some() {
            // The distinction that matters: a page that did not move (scrollbar delta 0)
            // versus a capture that is frozen while the scrollbar moves (the window
            // server hands back the same picture — locked screen, screenshot shield).
            eprintln!(
                "live: frame={}x{} fraction={fraction:.4} delta={delta:?} probe={probed:?} ppu={px_per_unit:?} fp={fingerprint:016x} identical_to_previous={frozen} scrollbar_moved={scrollbar_moved}{}",
                frame.width,
                frame.height,
                if frozen && scrollbar_moved {
                    "  <- CAPTURE FROZEN while the scrollbar moves"
                } else {
                    ""
                }
            );
            previous_fingerprint = Some(fingerprint);
        }

        // The scrollbar is the reliable measurement: it is a number the app itself
        // updated, while a repetitive page can give the pixel matcher a confident but
        // wrong offset. The matcher is used only to calibrate pixels-per-unit and to
        // refine it when both agree.
        if let (Some(per_unit), Some(delta)) = (px_per_unit, delta) {
            if let Probe::Moved(shift) = probed {
                let expected = delta.abs() as f32 * per_unit;
                if delta.abs() > 1e-4
                    && expected > 8.0
                    && (shift as f32 - expected).abs() <= expected * 0.35
                {
                    let refined = shift as f32 / delta.abs() as f32;
                    let blended = per_unit * 0.7 + refined * 0.3;
                    px_per_unit = Some(blended);
                }
            }
            let by_bar = (delta.abs() as f32 * per_unit).round() as usize;
            if by_bar == 0 {
                stalls += 1;
                continue;
            }
            if by_bar + 8 >= viewport_u32 as usize {
                // Asked for more than a viewport. The scroll at the top of the loop
                // already moved the page, so undo it before retrying — exactly like the
                // Probe::Unmatched branch below. Without the rollback the stitcher's
                // `last` stays on the pre-jump frame and the next append writes pixels
                // from the wrong page position, leaving a silent gap in the long image.
                // Propagate a failed rollback: continuing from a page position we only
                // *believe* we restored silently leaves a gap in the long image.
                scroll(-fraction * sign)?;
                wait();
                // The fingerprint recorded above belongs to the frame we just rolled
                // back; keeping it would skew the freeze check on the next iteration.
                previous_fingerprint = None;
                misses += 1;
                fraction *= 0.5;
                if fraction < 0.0005 {
                    break;
                }
                continue;
            }
            // When the pixel matcher agrees with the scrollbar it is the more precise of
            // the two (the scrollbar delta is the value *requested*, not the one the app
            // landed on), so prefer it for this step.
            let shift = match probed {
                Probe::Moved(measured)
                    if (measured as f32 - by_bar as f32).abs() <= by_bar as f32 * 0.35 =>
                {
                    measured.max(4)
                }
                _ => by_bar,
            };
            let strip = stitcher.append(&frame, shift)?;
            steps += 1;
            stalls = 0;
            misses = 0;
            let per_fraction = shift as f32 / fraction.max(0.001);
            let wanted = (target / per_fraction.max(1.0)).clamp(0.005, 0.6);
            fraction = wanted.min(fraction * 3.0);
            on_append(&strip, stitcher.height());
            continue;
        }

        match probed {
            Probe::Moved(shift) => {
                if px_per_unit.is_none() {
                    if let Some(delta) = delta {
                        if delta.abs() > 1e-4 {
                            px_per_unit = Some(shift as f32 / delta.abs() as f32);
                        }
                    }
                }
                let strip = stitcher.append(&frame, shift)?;
                steps += 1;
                stalls = 0;
                misses = 0;
                let per_fraction = shift as f32 / fraction.max(0.001);
                let wanted = (target / per_fraction.max(1.0)).clamp(0.005, 0.6);
                fraction = wanted.min(fraction * 3.0);
                on_append(&strip, stitcher.height());
            }
            Probe::Unmatched => {
                // Scrolled past the overlap. Undo the step first: the stitcher still
                // holds the previous frame, so retrying from anywhere else can never
                // match again. Then shrink and try once more.
                scroll(-fraction * sign)?;
                wait();
                misses += 1;
                fraction *= 0.5;
                if fraction < 0.0005 {
                    break;
                }
            }
            Probe::Still => {
                // "Still" only means "the page ended" when the scrollbar also stayed
                // put. A moving scrollbar with a failed match is the bootstrap case:
                // the first step is too large (or the content too repetitive) to match,
                // and treating it as the end is what made the gesture stop after one
                // frame. Shrink and try again instead.
                let moving = delta.map(|value| value.abs() > 1e-4).unwrap_or(false);
                if moving {
                    misses += 1;
                    fraction *= 0.4;
                    if fraction < 0.0005 {
                        break;
                    }
                } else {
                    stalls += 1;
                }
            }
        }
    }
    Ok(stitcher.image().clone())
}

/// What one scroll attempt did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollStep {
    /// Scrolled; the delta is the scrollbar move (0..1) when the app reports one.
    Moved(Option<f64>),
    /// This window exposes no scrollbar we can drive. Not an error: the capture falls
    /// back to watching the user scroll.
    Unsupported,
}

/// One hash per row: enough to match rows between two frames without running the
/// stitcher, which refuses pairs it is not confident about (exactly the pairs an upward
/// scroll produces).
fn row_hashes(image: &Image) -> Vec<u64> {
    let width = image.width as usize;
    let mut out = Vec::with_capacity(image.height as usize);
    for y in 0..image.height as usize {
        let base = y * width * 4;
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        let step = (width / 64).max(1);
        for x in (0..width).step_by(step) {
            let index = base + x * 4;
            // All three channels: the red one alone is often constant down a column.
            for channel in 0..3 {
                // Quantise: two captures of the same row differ by a level or two because
                // of dithering, and an exact hash would then match nothing.
                hash ^= (image.rgba[index + channel] >> 3) as u64;
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        out.push(hash);
    }
    out
}

/// How many rows line up when `b` is `a` scrolled down by `shift` (content moved up).
fn matched_rows(a: &[u64], b: &[u64], shift: usize) -> usize {
    let len = a.len().min(b.len());
    if shift >= len {
        return 0;
    }
    (0..len - shift).filter(|&y| a[y + shift] == b[y]).count()
}

/// Which way the page moved between two frames, from row alignment only.
///
/// +1 = content moved up (the user scrolled down), -1 = content moved down (scrolled
/// up), `None` when neither direction lines up (too little overlap, or nothing moved).
pub fn scroll_direction(previous: &Image, next: &Image) -> Option<i8> {
    if previous.width != next.width || previous.height != next.height {
        return None;
    }
    let a = row_hashes(previous);
    let b = row_hashes(next);
    let limit = a.len().saturating_sub(8).max(2);
    let step = (limit / 60).max(1);
    let mut best_down = (0usize, 0usize);
    let mut best_up = (0usize, 0usize);
    for shift in (step..limit).step_by(step) {
        let down = matched_rows(&a, &b, shift);
        if down > best_down.1 {
            best_down = (shift, down);
        }
        let up = matched_rows(&b, &a, shift);
        if up > best_up.1 {
            best_up = (shift, up);
        }
    }
    // A real match covers a good share of the overlap; noise does not.
    // The overlap shrinks with the shift, so the bar is a share of what could match.
    let threshold = 12usize;
    match (best_down.1 >= threshold, best_up.1 >= threshold) {
        (true, true) => {
            if best_down.1 > best_up.1 {
                Some(1)
            } else if best_up.1 > best_down.1 {
                Some(-1)
            } else {
                None
            }
        }
        (true, false) => Some(1),
        (false, true) => Some(-1),
        (false, false) => None,
    }
}

/// A cheap but discriminative fingerprint of a frame: FNV-1a over sampled bytes.
///
/// A mean would be useless here — two different pages often average to the same value,
/// while this changes as soon as any sampled pixel changes.
pub fn frame_fingerprint(image: &Image) -> u64 {
    let step = (image.rgba.len() / 16_384).max(1);
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for index in (0..image.rgba.len()).step_by(step) {
        hash ^= image.rgba[index] as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Displacement between two consecutive viewports, in pixels.
///
/// Runs the stitcher over just the pair, so the calibration uses exactly the same
/// matcher that will compose the final image.
pub fn measure_shift(before: &Image, after: &Image) -> Option<usize> {
    let width = before.width as usize;
    let height = before.height as usize;
    if width == 0 || height == 0 || after.width as usize != width || after.height as usize != height
    {
        return None;
    }
    let a = luma_from_rgba(&before.rgba, width, height)?;
    let b = luma_from_rgba(&after.rgba, width, height)?;
    let stitched = stitch_plan(&[a, b], &StitchOptions::default()).ok()?;
    stitched.offsets.first().copied()
}

/// Manual mode: the user scrolls, we only capture.
///
/// This is the guaranteed fallback from `spec/04-scrolling-capture.md` §11 — it works
/// on pages that ignore synthetic wheels (maps, remote desktops, canvas apps).
// `settle` is separate from `wait` on purpose: the tests use `wait` to model "the user
// scrolled once", and reusing it for a short pause would move their synthetic page.
#[allow(clippy::too_many_arguments)]
pub fn manual_scroll<C, W, S, Q, U, P>(
    mut capture: C,
    mut wait: W,
    should_stop: S,
    options: ScrollRunOptions,
    mut position: Q,
    mut settle: U,
    mut progress: P,
) -> Result<Image>
where
    C: FnMut() -> Result<Image>,
    W: FnMut(),
    S: Fn() -> bool,
    U: FnMut(),
    // Current scrollbar position (0 = top), when the caller can read one. It is what
    // makes the frame order reliable: the picture alone cannot tell whether the user
    // scrolled up or down when the content is repetitive.
    Q: FnMut() -> Option<f64>,
    P: FnMut(ScrollProgress),
{
    let stitch_options = StitchOptions {
        max_height: options.max_height,
        ..Default::default()
    };
    let first = capture()?;
    let mut collector = ScrollCollector::new(stitch_options, options.max_frames);
    // The user may scroll either way; remember which, so the frames can be put in
    // reading order before stitching.
    // Votes on the scroll direction, taken from the first few accepted pairs: one
    // ambiguous pair (cursor blink) must not decide the order of the whole image.
    let mut up_votes = 0usize;
    let mut down_votes = 0usize;
    let mut first_position: Option<f64> = None;
    let mut last_position: Option<f64> = None;
    let mut previous = first.clone();
    let mut estimate = first.height;
    progress(ScrollProgress {
        frames: 1,
        width: first.width,
        height: estimate,
        skipped: 0,
    });
    collector.push(first);

    // Frames that reveal nothing (the user paused, or the page ended) must not keep the
    // capture alive forever: after ~5 s of stillness, finish with what we have.
    let mut idle = 0u32;
    let mut skipped = 0usize;
    while collector.len() < options.max_frames && !should_stop() {
        wait();
        // One grab per tick. Sampling slowly enough to "settle" made consecutive frames
        // land several screens apart, which is exactly the case the stitcher cannot
        // handle; a frame taken mid-animation is still a valid snapshot.
        let _ = &mut settle;
        let frame = capture()?;
        let started = std::time::Instant::now();
        if let Some(value) = position() {
            if first_position.is_none() {
                first_position = Some(value);
            }
            last_position = Some(value);
        }
        let (width, height) = (frame.width, frame.height);
        // Measured before the frame is handed to the collector (which takes ownership).
        let direction_probe = match scroll_direction(&previous, &frame) {
            Some(1) => Some(false),
            Some(-1) => Some(true),
            _ => None,
        };
        // A frame that lines up in neither direction moved further than a viewport: the
        // caller asks the user to slow down, and the stitcher will drop it.
        if direction_probe.is_none() && !nearly_identical(&previous, &frame) {
            skipped += 1;
        }
        let outcome = collector.push(frame);
        if std::env::var_os("PINSHOT_TRACE").is_some() {
            eprintln!(
                "manual_scroll: capture {}ms frame {width}×{height} -> {outcome:?} len={}",
                started.elapsed().as_millis(),
                collector.len()
            );
        }
        match outcome {
            PushOutcome::Accepted => {
                match direction_probe {
                    Some(true) => up_votes += 1,
                    Some(false) => down_votes += 1,
                    None => {}
                }
                if let Some(last) = collector.frames().last() {
                    previous = last.clone();
                }
                estimate = estimate.saturating_add(height / 2);
                progress(ScrollProgress {
                    frames: collector.len(),
                    width,
                    height: estimate,
                    skipped,
                });
            }
            // A pause is not an end: give the user a few seconds before finishing on
            // their behalf, so a slow read does not cut the capture short.
            PushOutcome::Duplicate | PushOutcome::Stopped(_) => {
                idle += 1;
                if idle >= 20 {
                    break;
                }
            }
        }
    }
    let mut frames = collector.into_frames();
    // The scrollbar is the ground truth when we can read it; the pixel votes are only a
    // fallback for callers that cannot.
    let reverse = match (first_position, last_position) {
        (Some(start), Some(end)) if (end - start).abs() > 1e-4 => end < start,
        _ => up_votes > down_votes,
    };
    if reverse {
        frames.reverse();
    }
    let options = StitchOptions {
        max_height: options.max_height,
        ..Default::default()
    };
    stitch_images_tolerant(&frames, &options)
}

/// Mean absolute difference below one level per channel: same picture, maybe with
/// sensor noise. Sampling keeps this cheap on a 5K frame.
fn nearly_identical(a: &Image, b: &Image) -> bool {
    if a.width != b.width || a.height != b.height {
        return false;
    }
    let pixels = a.rgba.len() / 4;
    let step = (pixels / 20_000).max(1);
    let mut total = 0u64;
    let mut count = 0u64;
    for index in (0..pixels).step_by(step) {
        let offset = index * 4;
        for channel in 0..3 {
            total += a.rgba[offset + channel].abs_diff(b.rgba[offset + channel]) as u64;
        }
        count += 3;
    }
    count > 0 && (total as f64 / count as f64) < 1.0
}

fn map_error(error: StitchError, _frames: usize) -> Error {
    match error {
        StitchError::LowConfidence { frame, score, .. } => Error::internal(format!(
            "滚动拼接失败：第 {frame} 帧的匹配度只有 {score:.2}，建议改用手动滚动"
        )),
        StitchError::NoOverlap { frame } => Error::internal(format!(
            "滚动拼接失败：第 {frame} 帧重叠区域不足（页面滚得太快）"
        )),
        other => Error::internal(format!("滚动拼接失败：{other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pinshot_stitch::StitchOptions;

    const WIDTH: u32 = 240;
    const VIEWPORT: u32 = 200;
    const HEADER: u32 = 24;
    const PAGE: u32 = 900;

    /// Every pixel encodes its own source row, so the composite can be verified by
    /// reading pixels instead of comparing images.
    fn page() -> Image {
        let mut image = Image::new(WIDTH, PAGE);
        for y in 0..PAGE {
            for x in 0..WIDTH {
                let index = (y as usize * WIDTH as usize + x as usize) * 4;
                image.rgba[index] = (x % 256) as u8;
                // Strong row-to-row contrast: a smooth gradient would make every row
                // look "fixed" to the sticky-header detector.
                image.rgba[index + 1] = ((y * 37) % 256) as u8;
                image.rgba[index + 2] = (y / 256) as u8;
                image.rgba[index + 3] = 255;
            }
        }
        image
    }

    fn viewport(source: &Image, scroll: u32) -> Image {
        let body = VIEWPORT - HEADER;
        let scroll = scroll.min(PAGE - body);
        let mut frame = Image::new(WIDTH, VIEWPORT);
        let row_bytes = WIDTH as usize * 4;
        // Sticky header: the same rows in every frame.
        frame.rgba[..HEADER as usize * row_bytes]
            .copy_from_slice(&source.rgba[..HEADER as usize * row_bytes]);
        for row in 0..body {
            let src = (scroll + row) as usize * row_bytes;
            let dst = (HEADER + row) as usize * row_bytes;
            frame.rgba[dst..dst + row_bytes].copy_from_slice(&source.rgba[src..src + row_bytes]);
        }
        frame
    }

    #[test]
    fn a_colour_page_comes_back_in_one_piece() {
        let source = page();
        let body = VIEWPORT - HEADER;
        let mut frames = Vec::new();
        let mut scroll = 0;
        loop {
            frames.push(viewport(&source, scroll));
            if scroll + body >= PAGE {
                break;
            }
            scroll = (scroll + 120).min(PAGE - body);
        }

        let stitched = stitch_images(&frames, &StitchOptions::default()).expect("stitch");

        assert_eq!(stitched.width, WIDTH);
        assert_eq!(stitched.height, HEADER + PAGE);
        // Header once, then the page, pixel for pixel.
        for y in 0..HEADER {
            for x in 0..WIDTH {
                assert_eq!(stitched.pixel(x, y), source.pixel(x, y), "header row {y}");
            }
        }
        for y in 0..PAGE {
            for x in [0, 17, WIDTH - 1] {
                assert_eq!(
                    stitched.pixel(x, HEADER + y),
                    source.pixel(x, y),
                    "page row {y} column {x}"
                );
            }
        }
    }

    #[test]
    fn unrelated_frames_are_reported_not_stiched() {
        let mut first = Image::new(WIDTH, VIEWPORT);
        let mut second = Image::new(WIDTH, VIEWPORT);
        for index in 0..first.rgba.len() {
            first.rgba[index] = (index * 37 % 251) as u8;
            second.rgba[index] = ((index + 11) * 91 % 253) as u8;
        }
        let error = stitch_images(&[first, second], &StitchOptions::default())
            .err()
            .unwrap();
        assert!(error.message.contains("滚动拼接失败"), "{}", error.message);
    }

    #[test]
    fn the_collector_stops_when_the_page_stops() {
        let source = page();
        let body = VIEWPORT - HEADER;
        let mut collector = ScrollCollector::new(StitchOptions::default(), 50);
        let mut scroll = 0;
        loop {
            let outcome = collector.push(viewport(&source, scroll));
            assert_eq!(outcome, PushOutcome::Accepted);
            if scroll + body >= PAGE {
                break;
            }
            scroll = (scroll + 120).min(PAGE - body);
        }
        // A resting frame is ignored; the second one ends the run.
        assert_eq!(
            collector.push(viewport(&source, PAGE - body)),
            PushOutcome::Duplicate
        );
        assert_eq!(
            collector.push(viewport(&source, PAGE - body)),
            PushOutcome::Stopped(StopReason::EndOfContent)
        );

        let stitched = collector.finish().unwrap();
        assert_eq!(stitched.height, HEADER + PAGE);
    }

    #[test]
    fn the_collector_honours_the_frame_limit() {
        let source = page();
        let mut collector = ScrollCollector::new(StitchOptions::default(), 3);
        let mut scroll = 0;
        for _ in 0..3 {
            assert_eq!(
                collector.push(viewport(&source, scroll)),
                PushOutcome::Accepted
            );
            scroll += 100;
        }
        assert_eq!(
            collector.push(viewport(&source, scroll)),
            PushOutcome::Stopped(StopReason::FrameLimit)
        );
        assert_eq!(collector.len(), 3);
    }

    #[test]
    fn auto_scroll_walks_a_page_to_the_end() {
        let source = page();
        let body = VIEWPORT - HEADER;
        // `Cell` lets both closures touch the offset: capture reads it, scroll moves it.
        let offset = std::cell::Cell::new(0u32);
        let mut steps = 0;

        let stitched = auto_scroll(
            || Ok(viewport(&source, offset.get())),
            |pixels| {
                steps += 1;
                offset.set((offset.get() + pixels).min(PAGE - body));
                Ok(())
            },
            || {},
            ScrollRunOptions::default(),
            |_| {},
        )
        .expect("auto scroll");

        assert!(steps >= 5, "should need several steps, took {steps}");
        assert_eq!(stitched.height, HEADER + PAGE);
        for y in [0, 100, 400, 800] {
            for x in [0, 120, WIDTH - 1] {
                assert_eq!(
                    stitched.pixel(x, HEADER + y),
                    source.pixel(x, y),
                    "row {y} column {x}"
                );
            }
        }
    }

    #[test]
    fn auto_scroll_upwards_stitches_in_the_right_order() {
        let source = page();
        let body = VIEWPORT - HEADER;
        // Start at the bottom and walk up, the way "drag to the top edge" works.
        let offset = std::cell::Cell::new(PAGE - body);

        let stitched = auto_scroll(
            || Ok(viewport(&source, offset.get())),
            |pixels| {
                offset.set(offset.get().saturating_sub(pixels));
                Ok(())
            },
            || {},
            ScrollRunOptions {
                direction: Direction::Up,
                step_ratio: 0.5,
                ..Default::default()
            },
            |_| {},
        )
        .expect("upward scroll");

        assert_eq!(stitched.height, HEADER + PAGE);
        for y in [0, 200, 500, 800] {
            for x in [0, 120, WIDTH - 1] {
                assert_eq!(
                    stitched.pixel(x, HEADER + y),
                    source.pixel(x, y),
                    "row {y} column {x}"
                );
            }
        }
    }

    #[test]
    fn auto_scroll_stops_when_the_page_cannot_move() {
        let source = page();
        let mut steps = 0;
        let stitched = auto_scroll(
            || Ok(viewport(&source, 0)),
            |_| {
                steps += 1;
                Ok(())
            },
            || {},
            ScrollRunOptions::default(),
            |_| {},
        )
        .expect("auto scroll");
        // Two identical frames end the run; the loop never runs away.
        assert!(steps <= 2, "took {steps} steps on a static page");
        assert_eq!(stitched.height, HEADER + VIEWPORT - HEADER);
    }

    #[test]
    fn manual_scroll_captures_what_the_user_scrolled() {
        let source = page();
        let body = VIEWPORT - HEADER;
        let offset = std::cell::Cell::new(0u32);
        let captures = std::cell::Cell::new(0u32);

        let stitched = manual_scroll(
            || {
                captures.set(captures.get() + 1);
                Ok(viewport(&source, offset.get()))
            },
            || {
                // The "user" scrolls less than a viewport per capture; 140 px would
                // leave only 36 rows of overlap and the stitcher needs 48.
                offset.set((offset.get() + 96).min(PAGE - body));
            },
            || offset.get() >= PAGE - body,
            ScrollRunOptions::default(),
            || Some(offset.get() as f64 / PAGE as f64),
            || {},
            |_| {},
        )
        .expect("manual scroll");

        assert!(captures.get() >= 4);
        assert_eq!(stitched.height, HEADER + PAGE);
    }

    #[test]
    fn live_stitcher_grows_a_page_as_it_scrolls() {
        let source = page();
        let body = VIEWPORT - HEADER;
        let mut stitcher = LiveStitcher::new(viewport(&source, 0), Direction::Down, 2);
        assert_eq!(stitcher.height(), VIEWPORT);

        let mut scroll = 0u32;
        let mut strips = 0;
        while scroll + body < PAGE {
            scroll = (scroll + 120).min(PAGE - body);
            let step = stitcher
                .push(&viewport(&source, scroll))
                .expect("live scroll");
            if matches!(step, LiveStep::Grown { .. }) {
                strips += 1;
            }
        }

        assert!(
            strips >= 4,
            "should have revealed several strips, got {strips}"
        );
        assert_eq!(stitcher.height(), HEADER + PAGE);
        for y in [0, 100, 400, 800] {
            for x in [0, 120, WIDTH - 1] {
                assert_eq!(
                    stitcher.image().pixel(x, HEADER + y),
                    source.pixel(x, y),
                    "row {y} column {x}"
                );
            }
        }
    }

    #[test]
    fn a_still_page_reveals_nothing() {
        let source = page();
        let mut stitcher = LiveStitcher::new(viewport(&source, 0), Direction::Down, 8);
        assert!(matches!(
            stitcher.push(&viewport(&source, 0)).unwrap(),
            LiveStep::Stalled
        ));
        assert_eq!(stitcher.height(), VIEWPORT);
    }

    #[test]
    fn live_stitcher_handles_upward_scrolling() {
        let source = page();
        let body = VIEWPORT - HEADER;
        let mut offset = PAGE - body;
        let mut stitcher = LiveStitcher::new(viewport(&source, offset), Direction::Up, 8);

        while offset > 0 {
            offset = offset.saturating_sub(100);
            stitcher
                .push(&viewport(&source, offset))
                .expect("upward live scroll");
        }

        assert_eq!(stitcher.height(), HEADER + PAGE);
        for y in [0, 200, 500, 800] {
            for x in [0, 120, WIDTH - 1] {
                assert_eq!(
                    stitcher.image().pixel(x, HEADER + y),
                    source.pixel(x, y),
                    "row {y} column {x}"
                );
            }
        }
    }

    #[test]
    fn a_frozen_capture_is_reported_not_treated_as_the_end() {
        // The window server can hand back the same picture while the scrollbar moves
        // (locked screen, screenshot shield). That used to look like "the page ended",
        // which is what made the gesture stop after one frame.
        let source = page();
        let frame = viewport(&source, 0);
        let error = live_scroll(
            || Ok(frame.clone()),
            |_| Ok(ScrollStep::Moved(Some(0.02))),
            || {},
            || false,
            Direction::Down,
            10,
            |_, _| {},
        )
        .expect_err("a frozen capture must be an error");
        assert!(error.message.contains("画面没有变化"), "{}", error.message);
    }

    #[test]
    fn scroll_direction_is_read_from_the_rows_alone() {
        let source = page();
        let down = viewport(&source, 240); // content moved up relative to `up`
        let up = viewport(&source, 120);

        // down after up = the user scrolled down, content moved up.
        assert_eq!(scroll_direction(&up, &down), Some(1));
        // down before up = the user scrolled up, content moved down.
        assert_eq!(scroll_direction(&down, &up), Some(-1));
        // Same picture: no direction.
        assert_eq!(scroll_direction(&up, &up.clone()), None);
    }

    #[test]
    fn a_frame_with_no_overlap_is_skipped_not_fatal() {
        let source = page();
        // 0 → 120 → 240 overlap; 760 is six screens away and cannot match; 880 is just
        // as far from what we kept.
        let frames: Vec<Image> = [0u32, 120, 240, 760, 880]
            .iter()
            .map(|&scroll| viewport(&source, scroll))
            .collect();
        let stitched =
            stitch_images_tolerant(&frames, &StitchOptions::default()).expect("tolerant stitch");
        // The jump must not abort the run: at least the frames that line up survive.
        assert!(
            stitched.height >= HEADER + 240 + (VIEWPORT - HEADER),
            "height {}",
            stitched.height
        );
        for y in [0, 100, 240] {
            assert_eq!(stitched.pixel(0, HEADER + y), source.pixel(0, y), "row {y}");
        }
    }

    #[test]
    fn mismatched_frame_sizes_are_rejected() {
        let frames = vec![Image::new(10, 10), Image::new(10, 12)];
        assert!(stitch_images(&frames, &StitchOptions::default()).is_err());
        assert!(stitch_images(&[], &StitchOptions::default()).is_err());
    }
}
