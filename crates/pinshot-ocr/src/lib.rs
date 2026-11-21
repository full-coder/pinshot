//! Text recognition (OCR) through the system Vision framework.
//!
//! Runs entirely on-device: no network, no model download, and the same engine the
//! system uses for Live Text. `spec/01-requirements.md` (OCR-1) asks for exactly this.

use pinshot_core::{Image, Result};

/// One recognised line, with Vision's normalised box (origin bottom-left).
#[derive(Clone, Debug, PartialEq)]
pub struct TextLine {
    pub text: String,
    pub confidence: f32,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl TextLine {
    /// Centre of the line in image pixels, handy for highlighting a hit.
    pub fn centre_px(&self, image_width: u32, image_height: u32) -> (f32, f32) {
        (
            (self.x + self.width / 2.0) * image_width as f32,
            (1.0 - (self.y + self.height / 2.0)) * image_height as f32,
        )
    }
}

/// Recognises text in an image. Empty when the image has none.
#[cfg(target_os = "macos")]
pub fn recognize(image: &Image) -> Result<Vec<TextLine>> {
    imp::recognize(image)
}

#[cfg(not(target_os = "macos"))]
pub fn recognize(_image: &Image) -> Result<Vec<TextLine>> {
    Err(pinshot_core::Error::unsupported(
        "OCR is only implemented on macOS (Vision); Linux/Windows land with M6",
    ))
}

/// Joins the lines into one searchable string, in reading order.
pub fn plain_text(lines: &[TextLine]) -> String {
    let mut sorted: Vec<&TextLine> = lines.iter().collect();
    // Vision returns observations in no guaranteed order; sort top-to-bottom, then
    // left-to-right, which is what "select all text" should produce.
    sorted.sort_by(|a, b| {
        let row_a = (1.0 - a.y).round_for_sort();
        let row_b = (1.0 - b.y).round_for_sort();
        row_a
            .partial_cmp(&row_b)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
    });
    sorted
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

trait RoundForSort {
    fn round_for_sort(&self) -> f32;
}

impl RoundForSort for f32 {
    fn round_for_sort(&self) -> f32 {
        // Group lines that sit on the same visual row (~1% of the page height).
        (self * 100.0).round() / 100.0
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::TextLine;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;

    use objc2_core_foundation::{CFRetained, CGRect};
    use objc2_core_graphics::{
        kCGColorSpaceSRGB, CGBitmapContextCreate, CGBitmapContextCreateImage, CGColorSpace, CGImage,
    };
    use objc2_foundation::{NSArray, NSDictionary, NSString};
    use objc2_vision::{
        VNImageOption, VNImageRequestHandler, VNRecognizeTextRequest, VNRequest,
        VNRequestTextRecognitionLevel,
    };
    use pinshot_core::Error;
    use pinshot_core::{Image, Result};
    use std::ffi::c_void;

    const RGBA8: u32 = 1 | (4 << 12);

    pub fn recognize(image: &Image) -> Result<Vec<TextLine>> {
        if image.is_empty() {
            return Ok(Vec::new());
        }
        let cg_image = to_cg_image(image)?;
        // Constructed through `msg_send!` rather than the generated `init` helper: the
        // binding's `alloc` is not exposed for this class, and the two-step
        // alloc/init form is the canonical Objective-C way anyway.
        let options: Retained<NSDictionary<VNImageOption, AnyObject>> = NSDictionary::new();
        let allocated: *mut AnyObject =
            unsafe { objc2::msg_send![objc2::class!(VNImageRequestHandler), alloc] };
        let handler_pointer: *mut VNImageRequestHandler =
            unsafe { objc2::msg_send![allocated, initWithCGImage: &*cg_image, options: &*options] };
        let handler = unsafe {
            Retained::from_raw(handler_pointer)
                .ok_or_else(|| Error::internal("cannot create the Vision request handler"))?
        };

        let request = {
            let request = VNRecognizeTextRequest::new();
            request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
            request.setUsesLanguageCorrection(true);
            let languages = NSArray::from_retained_slice(&[
                NSString::from_str("zh-Hans"),
                NSString::from_str("en-US"),
            ]);
            request.setRecognitionLanguages(&languages);
            request
        };

        let as_request = Retained::into_super(Retained::into_super(request.clone()));
        let requests: Retained<NSArray<VNRequest>> = NSArray::from_retained_slice(&[as_request]);
        handler
            .performRequests_error(&requests)
            .map_err(|error| Error::internal(format!("Vision OCR failed: {error}")))?;

        let mut lines = Vec::new();
        if let Some(results) = request.results() {
            for observation in results.iter() {
                let candidates = observation.topCandidates(1);
                let Some(best) = candidates.firstObject() else {
                    continue;
                };
                let text = best.string().to_string();
                if text.trim().is_empty() {
                    continue;
                }
                let box_rect: CGRect = unsafe { objc2::msg_send![&*observation, boundingBox] };
                lines.push(TextLine {
                    text,
                    confidence: best.confidence(),
                    x: box_rect.origin.x as f32,
                    y: box_rect.origin.y as f32,
                    width: box_rect.size.width as f32,
                    height: box_rect.size.height as f32,
                });
            }
        }
        Ok(lines)
    }

    /// Rebuilds a CGImage from the packed RGBA buffer.
    fn to_cg_image(image: &Image) -> Result<CFRetained<CGImage>> {
        let mut pixels = image.rgba.clone();
        let width = image.width as usize;
        let height = image.height as usize;
        let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))
            .ok_or_else(|| Error::internal("cannot create the sRGB colour space"))?;
        let context = unsafe {
            CGBitmapContextCreate(
                pixels.as_mut_ptr() as *mut c_void,
                width,
                height,
                8,
                width * 4,
                Some(&space),
                RGBA8,
            )
        }
        .ok_or_else(|| Error::internal("cannot create a bitmap context for OCR"))?;
        let cg = CGBitmapContextCreateImage(Some(&context))
            .ok_or_else(|| Error::internal("cannot snapshot the bitmap context"))?;
        Ok(cg)
    }

    /// Keeps the unused import honest across binding versions.
    #[allow(dead_code)]
    fn _unused(_: &AnyObject) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_sorts_by_row_then_column() {
        let line = |text: &str, x: f32, y: f32| TextLine {
            text: text.into(),
            confidence: 0.9,
            x,
            y,
            width: 0.1,
            height: 0.02,
        };
        let lines = vec![
            line("second-row", 0.1, 0.70),
            line("first-row-right", 0.60, 0.90),
            line("first-row-left", 0.10, 0.90),
        ];
        // Vision's y is bottom-up, so 0.90 is the top row.
        assert_eq!(
            plain_text(&lines),
            "first-row-left\nfirst-row-right\nsecond-row"
        );
    }

    #[test]
    fn centre_is_converted_to_pixels_top_down() {
        let line = TextLine {
            text: "x".into(),
            confidence: 1.0,
            x: 0.25,
            y: 0.75,
            width: 0.5,
            height: 0.1,
        };
        // y is bottom-up: 0.75..0.85 → top-down centre at 0.2 of the height.
        let (x, y) = line.centre_px(1000, 500);
        assert!((x - 500.0).abs() < 0.01);
        assert!((y - 100.0).abs() < 0.01);
    }
}
