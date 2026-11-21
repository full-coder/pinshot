//! A packed RGBA8 image. One type for capture, stitching output and export, so a
//! frame never has to be converted between crates.

use crate::geom::Rect;

#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// Packed RGBA, row-major, 4 bytes per pixel, **not** premultiplied.
    pub rgba: Vec<u8>,
}

impl Image {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            rgba: vec![0; width as usize * height as usize * 4],
        }
    }

    /// Returns `None` when the buffer length does not match the dimensions: a
    /// half-filled frame must never reach the renderer.
    pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Option<Self> {
        if rgba.len() != width as usize * height as usize * 4 {
            return None;
        }
        Some(Self {
            width,
            height,
            rgba,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn byte_len(&self) -> usize {
        self.rgba.len()
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    #[inline]
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let index = (y as usize * self.width as usize + x as usize) * 4;
        [
            self.rgba[index],
            self.rgba[index + 1],
            self.rgba[index + 2],
            self.rgba[index + 3],
        ]
    }

    /// Crops to `rect`, clipped to the image. Returns an empty image when the
    /// rectangle falls outside — callers check `is_empty` instead of unwrapping.
    pub fn crop(&self, rect: Rect) -> Image {
        let Some(clipped) = rect.intersect(&self.bounds()) else {
            return Image::new(0, 0);
        };
        let mut out = Image::new(clipped.w, clipped.h);
        let row_bytes = clipped.w as usize * 4;
        for row in 0..clipped.h {
            let src_start =
                ((clipped.y as u32 + row) as usize * self.width as usize + clipped.x as usize) * 4;
            let dst_start = row as usize * row_bytes;
            out.rgba[dst_start..dst_start + row_bytes]
                .copy_from_slice(&self.rgba[src_start..src_start + row_bytes]);
        }
        out
    }

    /// Memory used by the pixel buffer.
    pub fn memory(&self) -> usize {
        self.rgba.capacity()
    }
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print 60 MB of pixels in a log line.
        f.debug_struct("Image")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.rgba.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(width: u32, height: u32) -> Image {
        let mut image = Image::new(width, height);
        for y in 0..height {
            for x in 0..width {
                let index = (y as usize * width as usize + x as usize) * 4;
                image.rgba[index] = x as u8;
                image.rgba[index + 1] = y as u8;
                image.rgba[index + 2] = 7;
                image.rgba[index + 3] = 255;
            }
        }
        image
    }

    #[test]
    fn rejects_mismatched_buffers() {
        assert!(Image::from_rgba(2, 2, vec![0; 15]).is_none());
        assert!(Image::from_rgba(2, 2, vec![0; 16]).is_some());
    }

    #[test]
    fn crop_keeps_the_requested_window() {
        let image = gradient(8, 6);
        let cropped = image.crop(Rect::new(2, 1, 3, 2));
        assert_eq!((cropped.width, cropped.height), (3, 2));
        assert_eq!(cropped.pixel(0, 0), [2, 1, 7, 255]);
        assert_eq!(cropped.pixel(2, 1), [4, 2, 7, 255]);
    }

    #[test]
    fn crop_clips_to_the_image() {
        let image = gradient(4, 4);
        let cropped = image.crop(Rect::new(3, 3, 10, 10));
        assert_eq!((cropped.width, cropped.height), (1, 1));
        assert!(image.crop(Rect::new(9, 9, 2, 2)).is_empty());
    }
}
