//! Throughput check for the displacement search on a realistic viewport.
//!
//! ```bash
//! cargo run --release --example bench -p pinshot-stitch
//! ```
//!
//! Budget from `spec/08-performance.md`: one frame pair on a 1440×900 viewport must
//! stay under 12 ms, so a 10-screen page (~20 pairs) finishes in ~250 ms plus capture.

use pinshot_stitch::{stitch, Gray, StitchOptions};
use std::time::Instant;

const WIDTH: usize = 1440;
const VIEWPORT: usize = 900;
const HEADER: usize = 64;
const PAGE_HEIGHT: usize = 12_000;
const STEP: usize = 630;

const LINE: usize = 18;

/// Text-like content: 18 px lines with a 6 px gap, 7 px "glyphs", and a unique solid
/// block per line so no two rows look alike even after a 4× box downsample.
fn page_value(x: usize, y: usize) -> u8 {
    let line = y / LINE;
    if y % LINE >= 12 {
        return 238;
    }
    let marker = (line * 37) % (WIDTH - 60);
    if x >= marker && x < marker + 40 {
        return 24;
    }
    let cell = x / 7;
    let seed = (line
        .wrapping_mul(2_654_435_761)
        .wrapping_add(cell.wrapping_mul(97)))
        % 1000;
    match seed % 6 {
        0 => 40,
        1 => 110,
        _ => 225,
    }
}

fn main() {
    let body = VIEWPORT - HEADER;
    let mut page = Gray::new(WIDTH, PAGE_HEIGHT);
    for y in 0..PAGE_HEIGHT {
        for x in 0..WIDTH {
            page.set(x, y, page_value(x, y));
        }
    }

    let mut scroll = 0usize;
    let mut frames = Vec::new();
    while scroll + body < PAGE_HEIGHT {
        let mut frame = Gray::new(WIDTH, VIEWPORT);
        frame.copy_rows(0, &page, 0, HEADER);
        frame.copy_rows(HEADER, &page, scroll, body);
        frames.push(frame);
        scroll += STEP;
    }
    // The last frame shows the page bottom.
    let mut frame = Gray::new(WIDTH, VIEWPORT);
    frame.copy_rows(0, &page, 0, HEADER);
    frame.copy_rows(HEADER, &page, PAGE_HEIGHT - body, body);
    frames.push(frame);

    let started = Instant::now();
    let result = stitch(&frames, &StitchOptions::default()).expect("stitch");
    let elapsed = started.elapsed();

    let pairs = frames.len() - 1;
    println!("viewport        {WIDTH}×{VIEWPORT}");
    println!("frames          {}", frames.len());
    println!("page height     {PAGE_HEIGHT}");
    println!("stitched height {}", result.height());
    println!(
        "per pair        {:.2} ms",
        elapsed.as_secs_f64() * 1000.0 / pairs as f64
    );
    println!("total           {:.1} ms", elapsed.as_secs_f64() * 1000.0);
    println!("offsets         {:?}", result.offsets);
}
