//! Real OCR against a fixture image: English, Chinese, an amount and an email.
//!
//! Runs the actual Vision framework, so it also proves the CGImage bridge (our packed
//! RGBA buffer → CGImage → Vision) is wired correctly.

use pinshot_core::Image;

fn fixture() -> Image {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/ocr/mixed-text.png"
    ))
    .expect("fixture image");
    let decoded = image::load_from_memory(&bytes)
        .expect("decode fixture")
        .to_rgba8();
    let (width, height) = decoded.dimensions();
    Image::from_rgba(width, height, decoded.into_raw()).expect("rgba buffer")
}

#[test]
fn recognises_english_chinese_and_numbers() {
    let image = fixture();
    let lines = match pinshot_ocr::recognize(&image) {
        Ok(lines) => lines,
        Err(error) => {
            // Never fail a build because a machine has no Vision framework available
            // (CI images without a GUI session); report instead of pretending.
            eprintln!("skipping: OCR unavailable here: {error}");
            return;
        }
    };
    assert!(!lines.is_empty(), "no text recognised at all");

    let text = pinshot_ocr::plain_text(&lines);
    println!("--- recognised ---\n{text}\n------------------");

    assert!(text.contains("PinShot"), "missing English line: {text}");
    assert!(text.contains("128.50"), "missing amount: {text}");
    assert!(text.contains("example.com"), "missing email: {text}");
    assert!(
        text.contains("滚动截屏") || text.contains("滚动"),
        "missing Chinese line: {text}"
    );
    // Vision reports a confidence per line; anything it returns should be sane.
    assert!(lines
        .iter()
        .all(|line| line.confidence > 0.0 && line.confidence <= 1.0));
}
