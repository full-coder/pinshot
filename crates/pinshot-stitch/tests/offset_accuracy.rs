//! Accuracy check with a text-like page: the gradient page in the unit tests is far
//! easier to align than real glyphs, and it never has runs of blank rows.

use pinshot_stitch::{stitch, Gray, StitchOptions};

const WIDTH: usize = 600;
const HEIGHT: usize = 400;
const PAGE: usize = 2000;

fn page() -> Gray {
    let mut image = Gray::new(WIDTH, PAGE);
    // White page with 1px glyph strokes, 12px lines and 12px gaps, plus pseudo-random
    // ink so no two lines look alike (a periodic page would hide alignment errors).
    let mut seed = 0x1234_5678u32;
    for y in 0..PAGE {
        let line = y % 24;
        for x in 0..WIDTH {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let ink = line < 12 && !(seed >> 24).is_multiple_of(5);
            image.set(x, y, if ink { 20 } else { 240 });
        }
    }
    image
}

fn viewport(source: &Gray, scroll: usize) -> Gray {
    let mut frame = Gray::new(WIDTH, HEIGHT);
    for row in 0..HEIGHT {
        for x in 0..WIDTH {
            frame.set(x, row, source.px(x, scroll + row));
        }
    }
    frame
}

#[test]
fn a_text_page_with_odd_offsets_is_pixel_exact() {
    let source = page();
    let offsets = [0, 137, 290, 433, 590, 731, 886, 1033, 1190, 1341];
    let frames: Vec<Gray> = offsets.iter().map(|&o| viewport(&source, o)).collect();
    let stitched = stitch(&frames, &StitchOptions::default()).expect("stitch");

    assert_eq!(
        stitched.image.height as usize,
        offsets[offsets.len() - 1] + HEIGHT
    );
    for y in 0..stitched.image.height as usize {
        for x in 0..WIDTH {
            assert_eq!(
                stitched.image.px(x, y),
                source.px(x, y),
                "page row {y} column {x}"
            );
        }
    }
}

#[test]
fn a_blank_band_at_the_seam_is_content_not_a_fixed_footer() {
    let mut source = page();
    // A 110-row empty block: every frame that ends inside it has a run of identical
    // blank rows at its bottom. Those are page whitespace and must not be mistaken for
    // a pinned footer (that trimmed a few rows and duplicated a line at the seam).
    for y in 400..510 {
        for x in 0..WIDTH {
            source.set(x, y, 240);
        }
    }
    let offsets = [0, 148, 301, 452, 605, 733, 860, 990, 1120, 1250];
    let frames: Vec<Gray> = offsets.iter().map(|&o| viewport(&source, o)).collect();
    let stitched = stitch(&frames, &StitchOptions::default()).expect("stitch");

    for y in 0..stitched.image.height as usize {
        for x in 0..WIDTH {
            assert_eq!(
                stitched.image.px(x, y),
                source.px(x, y),
                "page row {y} column {x}"
            );
        }
    }
}
