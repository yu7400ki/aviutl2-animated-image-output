use std::fmt;
use taffy::TaffyError;
use windows::core::Error as WindowsError;

#[derive(Debug)]
pub enum DialogError {
    /// Win32 API errors
    Win32Error(WindowsError),
    /// Layout calculation errors
    LayoutError(String),
    /// Widget creation or manipulation errors
    WidgetError(String),
    /// Invalid operations or usage errors
    InvalidOperation(String),
}

impl fmt::Display for DialogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DialogError::Win32Error(err) => write!(f, "Win32 API error: {}", err),
            DialogError::LayoutError(msg) => write!(f, "Layout error: {}", msg),
            DialogError::WidgetError(msg) => write!(f, "Widget error: {}", msg),
            DialogError::InvalidOperation(msg) => write!(f, "Invalid operation: {}", msg),
        }
    }
}

impl std::error::Error for DialogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DialogError::Win32Error(err) => Some(err),
            _ => None,
        }
    }
}

/// Automatic conversion from Windows API errors to DialogError
impl From<WindowsError> for DialogError {
    fn from(err: WindowsError) -> Self {
        DialogError::Win32Error(err)
    }
}

/// Automatic conversion from Taffy errors to DialogError
impl From<TaffyError> for DialogError {
    fn from(err: TaffyError) -> Self {
        DialogError::LayoutError(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, DialogError>;
