//! Integer geometry in two spaces: image pixels and screen points.
//!
//! `Rect` is half-open: it covers `x .. x + w` and `y .. y + h`. Every operation
//! here is saturating — a selection dragged past a screen edge must clamp, not panic.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Size {
    pub w: u32,
    pub h: u32,
}

impl Size {
    pub const fn new(w: u32, h: u32) -> Self {
        Self { w, h }
    }

    pub const fn is_empty(self) -> bool {
        self.w == 0 || self.h == 0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    /// Normalised rectangle between two dragged corners, in the same order the user
    /// dragged them. Dragging up/left must work exactly like dragging down/right.
    pub fn from_corners(a: Point, b: Point) -> Self {
        let x = a.x.min(b.x);
        let y = a.y.min(b.y);
        let w = a.x.abs_diff(b.x);
        let h = a.y.abs_diff(b.y);
        Self { x, y, w, h }
    }

    pub const fn size(&self) -> Size {
        Size::new(self.w, self.h)
    }

    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    pub fn right(&self) -> i32 {
        self.x.saturating_add(self.w as i32)
    }

    pub fn bottom(&self) -> i32 {
        self.y.saturating_add(self.h as i32)
    }

    pub fn contains(&self, point: Point) -> bool {
        point.x >= self.x && point.y >= self.y && point.x < self.right() && point.y < self.bottom()
    }

    pub fn union(&self, other: &Rect) -> Rect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Rect::new(x, y, (right - x) as u32, (bottom - y) as u32)
    }

    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        if right <= x || bottom <= y {
            return None;
        }
        Some(Rect::new(x, y, (right - x) as u32, (bottom - y) as u32))
    }

    pub fn offset(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(
            self.x.saturating_add(dx),
            self.y.saturating_add(dy),
            self.w,
            self.h,
        )
    }

    /// Shrinks the rectangle inwards on every side, never below empty.
    pub fn inset(&self, px: i32) -> Rect {
        let dx = px.min(self.w as i32 / 2);
        let dy = px.min(self.h as i32 / 2);
        Rect::new(
            self.x.saturating_add(dx),
            self.y.saturating_add(dy),
            (self.w as i32 - dx * 2).max(0) as u32,
            (self.h as i32 - dy * 2).max(0) as u32,
        )
    }

    pub fn area(&self) -> u64 {
        self.w as u64 * self.h as u64
    }
}

impl fmt::Display for Rect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}×{} at ({}, {})", self.w, self.h, self.x, self.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_normalise_any_drag_direction() {
        let a = Rect::from_corners(Point::new(100, 120), Point::new(40, 20));
        assert_eq!(a, Rect::new(40, 20, 60, 100));
        assert_eq!(
            a,
            Rect::from_corners(Point::new(40, 20), Point::new(100, 120))
        );
    }

    #[test]
    fn intersect_and_union() {
        let a = Rect::new(0, 0, 100, 100);
        let b = Rect::new(50, 50, 100, 100);
        assert_eq!(a.intersect(&b), Some(Rect::new(50, 50, 50, 50)));
        assert_eq!(a.union(&b), Rect::new(0, 0, 150, 150));
        assert_eq!(a.intersect(&Rect::new(200, 0, 10, 10)), None);
    }

    #[test]
    fn inset_never_underflows() {
        let r = Rect::new(0, 0, 4, 2);
        assert_eq!(r.inset(10), Rect::new(2, 1, 0, 0));
        assert!(r.inset(10).is_empty());
    }

    #[test]
    fn contains_uses_half_open_bounds() {
        let r = Rect::new(10, 10, 10, 10);
        assert!(r.contains(Point::new(10, 10)));
        assert!(r.contains(Point::new(19, 19)));
        assert!(!r.contains(Point::new(20, 19)));
    }
}
