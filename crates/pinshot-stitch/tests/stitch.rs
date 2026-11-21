//! Synthetic-frame tests for the stitcher.
//!
//! Every case builds a known page, renders the frames a real capture would produce
//! (sticky header, fixed sidebar, sticky footer, noise, brightness drift), and checks
//! that the composite is the original page again. No screen, no fixtures, no timing.

use pinshot_stitch::{stitch, Gray, StitchError, StitchOptions};

const PAGE_WIDTH: usize = 360;
const PAGE_HEIGHT: usize = 1_200;
const VIEWPORT: usize = 300;
const HEADER: usize = 30;
const STEP: usize = 200;

/// High-frequency, row-unique texture: neighbouring rows never look alike, so a wrong
/// offset cannot score well by accident.
fn page_value(x: usize, y: usize) -> u8 {
    ((x * 31 + y * 17 + (x * y) % 97 * 7) % 200 + 25) as u8
}

fn page(width: usize, height: usize) -> Gray {
    let mut gray = Gray::new(width, height);
    for y in 0..height {
        for x in 0..width {
            gray.set(x, y, page_value(x, y));
        }
    }
    gray
}

struct Scene {
    page: Gray,
    header: usize,
    footer: usize,
    sidebar: usize,
}

impl Scene {
    fn new(header: usize, footer: usize, sidebar: usize) -> Self {
        Self {
            page: page(PAGE_WIDTH, PAGE_HEIGHT),
            header,
            footer,
            sidebar,
        }
    }

    fn body(&self) -> usize {
        VIEWPORT - self.header - self.footer
    }

    /// What the user sees at a given scroll position (clamped at the page end).
    fn frame(&self, scroll: usize) -> Gray {
        let body = self.body();
        let scroll = scroll.min(PAGE_HEIGHT.saturating_sub(body));
        let mut out = Gray::new(PAGE_WIDTH, VIEWPORT);

        if self.header > 0 {
            out.copy_rows(0, &self.page, 0, self.header);
        }
        out.copy_rows(self.header, &self.page, scroll, body);

        // A fixed sidebar: same pixels in every frame, whatever the scroll offset.
        if self.sidebar > 0 {
            for y in self.header..self.header + body {
                for x in 0..self.sidebar {
                    out.set(x, y, ((x * 7 + 90) % 120 + 40) as u8);
                }
            }
        }

        // A sticky footer / status bar: also constant across frames.
        if self.footer > 0 {
            for y in VIEWPORT - self.footer..VIEWPORT {
                for x in 0..PAGE_WIDTH {
                    out.set(x, y, ((x * 5 + 60) % 120 + 40) as u8);
                }
            }
        }

        out
    }

    fn frames(&self, scrolls: &[usize]) -> Vec<Gray> {
        scrolls.iter().map(|&s| self.frame(s)).collect()
    }
}

/// Scroll offsets that walk the page to the end, then repeat the last one so the
/// stitcher observes "nothing moved" and stops.
fn scrolls(scene: &Scene, step: usize) -> Vec<usize> {
    let max = PAGE_HEIGHT.saturating_sub(scene.body());
    let mut out = vec![0];
    let mut current = 0;
    while current < max {
        current = (current + step).min(max);
        out.push(current);
    }
    out.push(max);
    out
}

fn assert_band_equal(
    actual: &Gray,
    actual_top: usize,
    expected: &Gray,
    expected_top: usize,
    rows: usize,
    columns: std::ops::Range<usize>,
) {
    for row in 0..rows {
        for x in columns.clone() {
            let a = actual.px(x, actual_top + row);
            let b = expected.px(x, expected_top + row);
            assert_eq!(
                a, b,
                "pixel mismatch at x={x} row={row} (actual {actual_top}+{row} = {a}, expected {expected_top}+{row} = {b})"
            );
        }
    }
}

#[test]
fn stitches_a_long_page_with_a_sticky_header() {
    let scene = Scene::new(HEADER, 0, 0);
    let frames = scene.frames(&scrolls(&scene, STEP));

    let result = stitch(&frames, &StitchOptions::default()).expect("stitch");

    assert_eq!(result.offsets, vec![200, 200, 200, 200, 130]);
    assert_eq!(result.header_height, HEADER);
    assert_eq!(result.height(), HEADER + PAGE_HEIGHT);
    assert!(
        result.stalled,
        "the repeated last frame should stop the run"
    );
    assert!(!result.truncated);

    // Header appears exactly once, at the top.
    assert_band_equal(&result.image, 0, &scene.page, 0, HEADER, 0..PAGE_WIDTH);
    // The body is the original page, stitched back together.
    assert_band_equal(
        &result.image,
        HEADER,
        &scene.page,
        0,
        PAGE_HEIGHT,
        0..PAGE_WIDTH,
    );
}

#[test]
fn a_fixed_sidebar_does_not_bias_the_offset() {
    let scene = Scene::new(HEADER, 0, 60);
    let frames = scene.frames(&scrolls(&scene, STEP));

    let result = stitch(&frames, &StitchOptions::default()).expect("stitch");

    assert_eq!(result.offsets, vec![200, 200, 200, 200, 130]);
    assert_eq!(result.height(), HEADER + PAGE_HEIGHT);
    // The sidebar is painted over the page in every frame, so only compare the
    // columns it does not cover.
    assert_band_equal(
        &result.image,
        HEADER,
        &scene.page,
        0,
        PAGE_HEIGHT,
        60..PAGE_WIDTH,
    );
}

#[test]
fn a_sticky_footer_is_dropped() {
    // A 24 px status bar leaves a 246 px body, so the step must shrink to keep
    // `min_overlap` rows shared between frames (46 px would be below the threshold).
    let scene = Scene::new(HEADER, 24, 0);
    let frames = scene.frames(&scrolls(&scene, 180));

    let result = stitch(&frames, &StitchOptions::default()).expect("stitch");

    assert_eq!(result.offsets, vec![180, 180, 180, 180, 180, 54]);
    assert_eq!(result.height(), HEADER + PAGE_HEIGHT);
    assert_band_equal(
        &result.image,
        HEADER,
        &scene.page,
        0,
        PAGE_HEIGHT,
        0..PAGE_WIDTH,
    );
    // The status bar pattern must not appear as a whole band anywhere. (A single
    // matching pixel proves nothing: the page texture repeats values.)
    let footer_row: Vec<u8> = (0..PAGE_WIDTH)
        .map(|x| ((x * 5 + 60) % 120 + 40) as u8)
        .collect();
    let leaked = (0..result.height().saturating_sub(24)).any(|top| {
        (0..24).all(|dy| (0..PAGE_WIDTH).all(|x| result.image.px(x, top + dy) == footer_row[x]))
    });
    assert!(!leaked, "a 24-row footer band leaked into the composite");
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// Uniform-ish integer in `-span..=span`.
    fn noise(&mut self, span: i32) -> i32 {
        (self.next() % (span as u64 * 2 + 1)) as i32 - span
    }
}

#[test]
fn survives_brightness_drift_and_sensor_noise() {
    let scene = Scene::new(HEADER, 0, 0);
    let mut rng = Rng(0x5EED_1234_ABCD_9876);
    let frames: Vec<Gray> = scrolls(&scene, STEP)
        .iter()
        .map(|&offset| {
            let clean = scene.frame(offset);
            let mut noisy = Gray::new(clean.width, clean.height);
            for y in 0..clean.height {
                for x in 0..clean.width {
                    // Fade-in animation: a global gain and lift, different per frame.
                    let base = clean.px(x, y) as i32;
                    let value = (base * 92 / 100 + 8 + rng.noise(5)).clamp(0, 255);
                    noisy.set(x, y, value as u8);
                }
            }
            noisy
        })
        .collect();

    let result = stitch(&frames, &StitchOptions::default()).expect("stitch");

    assert_eq!(result.offsets, vec![200, 200, 200, 200, 130]);
    assert_eq!(result.height(), HEADER + PAGE_HEIGHT);

    // Pixels differ by the noise, so compare with a tolerance instead of equality.
    let mut total = 0u64;
    let mut count = 0u64;
    for row in 0..PAGE_HEIGHT {
        for x in 0..PAGE_WIDTH {
            let a = result.image.px(x, HEADER + row) as i32;
            let b = scene.page.px(x, row) as i32;
            total += (a - b).unsigned_abs() as u64;
            count += 1;
        }
    }
    let mean = total as f64 / count as f64;
    assert!(mean < 6.0, "mean drift {mean:.2} is too large");
}

#[test]
fn stops_when_the_page_stops_moving() {
    let scene = Scene::new(HEADER, 0, 0);
    let frames = scene.frames(&[0, 200, 200, 200]);

    let result = stitch(&frames, &StitchOptions::default()).expect("stitch");

    assert_eq!(result.offsets, vec![200]);
    assert!(result.stalled);
    assert_eq!(result.height(), HEADER + scene.body() + 200);
}

#[test]
fn refuses_frames_it_cannot_align() {
    let mut rng = Rng(0xDEAD_BEEF_CAFE_0001);
    let mut frames = Vec::new();
    for _ in 0..3 {
        let mut gray = Gray::new(240, 200);
        for y in 0..200 {
            for x in 0..240 {
                gray.set(x, y, (rng.next() % 256) as u8);
            }
        }
        frames.push(gray);
    }

    match stitch(&frames, &StitchOptions::default()) {
        Err(StitchError::LowConfidence { frame, .. }) => assert_eq!(frame, 1),
        other => panic!("expected a low-confidence error, got {other:?}"),
    }
}

#[test]
fn truncates_at_the_height_limit() {
    let scene = Scene::new(HEADER, 0, 0);
    let frames = scene.frames(&scrolls(&scene, STEP));
    let options = StitchOptions {
        max_height: 700,
        ..Default::default()
    };

    let result = stitch(&frames, &options).expect("stitch");

    assert!(result.truncated);
    assert_eq!(result.height(), 700);
}

#[test]
fn rejects_frames_of_different_sizes() {
    let frames = vec![Gray::new(10, 10), Gray::new(10, 12)];
    assert_eq!(
        stitch(&frames, &StitchOptions::default()).unwrap_err(),
        StitchError::SizeMismatch
    );
    assert_eq!(
        stitch(&[], &StitchOptions::default()).unwrap_err(),
        StitchError::NoFrames
    );
}
