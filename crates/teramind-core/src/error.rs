use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("serialization failure: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("regex compile failure: {0}")]
    Regex(#[from] regex::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid input: {0}")]
    InvalidInput(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn error_from_io() {
        let e: Error = std::io::Error::new(std::io::ErrorKind::Other, "x").into();
        assert!(matches!(e, Error::Io(_)));
    }
}
