use std::fmt;

#[derive(Debug)]
pub enum HaioError {
    Config(String),
    Trojan(String),
    DomainFetch(String),
    Proxy(String),
    OsProxy(String),
    AppProxy(String),
    Io(std::io::Error),
    Serde(serde_json::Error),
    Reqwest(reqwest::Error),
    Other(String),
}

impl fmt::Display for HaioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HaioError::Config(msg) => write!(f, "Config error: {}", msg),
            HaioError::Trojan(msg) => write!(f, "Trojan error: {}", msg),
            HaioError::DomainFetch(msg) => write!(f, "Domain fetch error: {}", msg),
            HaioError::Proxy(msg) => write!(f, "Proxy error: {}", msg),
            HaioError::OsProxy(msg) => write!(f, "OS proxy error: {}", msg),
            HaioError::AppProxy(msg) => write!(f, "App proxy error: {}", msg),
            HaioError::Io(e) => write!(f, "IO error: {}", e),
            HaioError::Serde(e) => write!(f, "Serde error: {}", e),
            HaioError::Reqwest(e) => write!(f, "Reqwest error: {}", e),
            HaioError::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for HaioError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            HaioError::Io(e) => Some(e),
            HaioError::Serde(e) => Some(e),
            HaioError::Reqwest(e) => Some(e),
            _ => None,
        }
    }
}

impl serde::Serialize for HaioError {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl From<std::io::Error> for HaioError {
    fn from(e: std::io::Error) -> Self {
        HaioError::Io(e)
    }
}

impl From<serde_json::Error> for HaioError {
    fn from(e: serde_json::Error) -> Self {
        HaioError::Serde(e)
    }
}

impl From<reqwest::Error> for HaioError {
    fn from(e: reqwest::Error) -> Self {
        HaioError::Reqwest(e)
    }
}

pub type Result<T> = std::result::Result<T, HaioError>;
