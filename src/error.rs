use std::fmt;

/// Domain-specific error variants for user-friendly messages.
/// We use anyhow::Result everywhere; these are for matching in main().
#[derive(Debug)]
pub enum ForgeError {
    NotAuthenticated,
    TokenExpired,
    NotFound(String),
    Forbidden(String),
    ApiError(u16, String),
}

impl fmt::Display for ForgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ForgeError::NotAuthenticated => write!(f, "Not authenticated. Run `forge login` first."),
            ForgeError::TokenExpired => {
                write!(f, "Token has expired. Run `forge login` to re-authenticate.")
            }
            ForgeError::NotFound(msg) => write!(f, "{msg}"),
            ForgeError::Forbidden(msg) => write!(f, "Access denied: {msg}"),
            ForgeError::ApiError(status, msg) => write!(f, "API error ({status}): {msg}"),
        }
    }
}

impl std::error::Error for ForgeError {}
