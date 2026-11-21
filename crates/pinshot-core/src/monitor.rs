//! A display as the window system sees it: bounds in points, pixels in the frame.
//!
//! macOS and Windows place windows in points (dip) but hand back image buffers in
//! device pixels. Mixing the two is the classic HiDPI bug, so both are carried here
//! explicitly and converted in exactly one place.

use crate::geom::{Point, Rect};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Monitor {
    /// Platform display id (`CGDirectDisplayID`, `HMONITOR`, X11 output).
    pub id: u32,
    pub name: String,
    /// Position and size in points, in the virtual desktop coordinate space.
    pub bounds: Rect,
    /// Device pixel ratio: 2.0 on Retina, 1.0 on a plain display.
    pub scale: f32,
    /// Size of a captured frame in pixels; `bounds.size * scale`, rounded.
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub primary: bool,
}

impl Monitor {
    pub fn pixel_size(&self) -> (u32, u32) {
        (self.pixel_width, self.pixel_height)
    }

    /// Converts a point in **this monitor's local points** to a pixel coordinate
    /// inside the captured frame.
    pub fn to_pixel(&self, local: Point) -> (i32, i32) {
        let scale = if self.scale > 0.0 { self.scale } else { 1.0 };
        (
            (local.x as f32 * scale).round() as i32,
            (local.y as f32 * scale).round() as i32,
        )
    }

    /// Converts a rectangle in local points to a pixel rectangle inside the frame.
    pub fn rect_to_pixel(&self, local: Rect) -> Rect {
        let (x, y) = self.to_pixel(Point::new(local.x, local.y));
        let (right, bottom) = self.to_pixel(Point::new(local.right(), local.bottom()));
        Rect::new(x, y, (right - x).max(0) as u32, (bottom - y).max(0) as u32)
    }

    /// Converts a global virtual-desktop point into this monitor's local points.
    pub fn global_to_local(&self, global: Point) -> Point {
        Point::new(global.x - self.bounds.x, global.y - self.bounds.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn retina() -> Monitor {
        Monitor {
            id: 1,
            name: "Built-in".into(),
            bounds: Rect::new(0, 0, 1440, 900),
            scale: 2.0,
            pixel_width: 2880,
            pixel_height: 1800,
            primary: true,
        }
    }

    #[test]
    fn points_map_to_pixels_on_retina() {
        assert_eq!(retina().to_pixel(Point::new(10, 20)), (20, 40));
        assert_eq!(
            retina().rect_to_pixel(Rect::new(10, 10, 100, 50)),
            Rect::new(20, 20, 200, 100)
        );
    }

    #[test]
    fn a_secondary_display_offset_is_removed() {
        let mut second = retina();
        second.bounds = Rect::new(1440, 0, 1920, 1080);
        assert_eq!(
            second.global_to_local(Point::new(1500, 40)),
            Point::new(60, 40)
        );
    }
}
