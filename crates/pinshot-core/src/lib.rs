//! Shared types: geometry, the RGBA image container, errors and configuration.
//!
//! The crate deliberately has no platform code and no IO, so it compiles in
//! milliseconds and can be used by unit tests, the CLI and the desktop shell alike.
//!
//! Coordinate conventions (see `spec/02-architecture.md`, ADR-2):
//!
//! * **Image pixels** — `Image`, `crop`, export. Integer pixels.
//! * **Points (dip)** — window placement, monitor bounds, mouse events.
//! * The only bridge between them is `Monitor::scale` and `Monitor::to_pixel`.

pub mod config;
pub mod error;
pub mod geom;
pub mod image;
pub mod monitor;

pub use config::Config;
pub use error::{Error, ErrorKind, Result};
pub use geom::{Point, Rect, Size};
pub use image::Image;
pub use monitor::Monitor;
