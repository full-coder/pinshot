//! Sticky bars and blank page rows.
//!
//! `detect_fixed_rows` cannot tell a blank row that belongs to a sticky bar from a
//! blank row of page content, so the detected header/footer height wobbles by a few
//! rows between pairs. Every band used to take its source row from a *different*
//! pair's region, which shifted the seam by those rows (the long image showed a
//! duplicated or missing line). The capture now uses one consensus region.

use pinshot_stitch::{stitch, Gray, StitchOptions};

const WIDTH: usize = 600;
const VIEWPORT: usize = 400;
const PAGE: usize = 3200;

fn text_page() -> Gray {
    let mut image = Gray::new(WIDTH, PAGE);
    let mut seed = 0x1234_5678u32;
    for y in 0..PAGE {
        // 12 rows of glyph-like ink and 12 blank rows: the blank runs next to a sticky
        // bar are what made the detected band height vary.
        let line = y % 24;
        for x in 0..WIDTH {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let ink = line < 12 && !(seed >> 24).is_multiple_of(5);
            image.set(x, y, if ink { 20 } else { 240 });
        }
    }
    image
}

fn frame(source: &Gray, scroll: usize, header: usize, footer: usize) -> Gray {
    let body = VIEWPORT - header - footer;
    let mut out = Gray::new(WIDTH, VIEWPORT);
    let scroll = scroll.min(PAGE - body);
    out.copy_rows(0, source, 0, header);
    out.copy_rows(header, source, scroll, body);
    for y in VIEWPORT - footer..VIEWPORT {
        for x in 0..WIDTH {
            out.set(x, y, ((x * 3 + 100) % 90 + 40) as u8);
        }
    }
    out
}

#[test]
fn blank_rows_next_to_sticky_bars_do_not_shift_seams() {
    let source = text_page();
    let header = 30;
    let footer = 24;
    let body = VIEWPORT - header - footer;
    let mut scrolls = vec![0];
    let mut scroll = 0;
    while scroll < PAGE - body {
        scroll = (scroll + 173).min(PAGE - body);
        scrolls.push(scroll);
    }
    let frames: Vec<Gray> = scrolls
        .iter()
        .map(|&scroll| frame(&source, scroll, header, footer))
        .collect();

    let stitched = stitch(&frames, &StitchOptions::default()).expect("stitch");

    assert_eq!(stitched.header_height, header);
    assert_eq!(stitched.height(), header + PAGE);
    for y in 0..PAGE {
        for x in 0..WIDTH {
            assert_eq!(
                stitched.image.px(x, header + y),
                source.px(x, y),
                "page row {y} column {x}"
            );
        }
    }
}
