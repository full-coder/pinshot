//! Typed errors with an actionable hint, because `CGWindowListCreateImage` returning
//! nil tells the user nothing (see `spec/02-architecture.md` §7).

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorKind {
    /// The OS refused: screen recording or accessibility permission is missing.
    Permission,
    /// The platform cannot do this at all (Wayland window capture, …).
    Unsupported,
    NotFound,
    Busy,
    Io,
    Internal,
}

impl ErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Permission => "permission",
            Self::Unsupported => "unsupported",
            Self::NotFound => "notFound",
            Self::Busy => "busy",
            Self::Io => "io",
            Self::Internal => "internal",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
    /// What the user can do about it, shown next to the message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            hint: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn permission(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Permission, message)
            .with_hint("打开「系统设置 → 隐私与安全性 → 屏幕录制」，允许 PinShot 后重试")
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Unsupported, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound, message)
    }

    pub fn io(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Io, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Internal, message)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.hint {
            Some(hint) => write!(f, "{} ({})", self.message, hint),
            None => write!(f, "{}", self.message),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::io(error.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_includes_the_hint() {
        let error = Error::permission("cannot capture");
        assert!(error.to_string().contains("系统设置"));
        assert_eq!(error.kind, ErrorKind::Permission);
    }
}
