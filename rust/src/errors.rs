//! Typed errors for the plugin. Every failure carries a machine-readable kind,
//! a human message, and optional captured output.

use thiserror::Error;

/// Machine-readable failure kinds, mirroring the `errorKind` result field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Startup,
    NotFound,
    Permission,
    Config,
    /// A Kata/containerd operation failed.
    Kata,
    /// The agent binary misbehaved.
    Agent,
    /// Not implemented yet (honest stub).
    Unimplemented,
    Unknown,
}

impl ErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorKind::Startup => "startup",
            ErrorKind::NotFound => "notFound",
            ErrorKind::Permission => "permission",
            ErrorKind::Config => "config",
            ErrorKind::Kata => "kata",
            ErrorKind::Agent => "agent",
            ErrorKind::Unimplemented => "unimplemented",
            ErrorKind::Unknown => "unknown",
        }
    }
}

/// A plugin failure.
#[derive(Debug, Error)]
#[error("{message}")]
pub struct PluginError {
    pub kind: ErrorKind,
    pub message: String,
    #[source]
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
    /// Captured stdout/stderr of a failed subprocess, if any.
    pub output: String,
}

impl PluginError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        PluginError {
            kind,
            message: message.into(),
            source: None,
            output: String::new(),
        }
    }

    pub fn with_output(mut self, output: impl Into<String>) -> Self {
        self.output = output.into();
        self
    }
}

pub type Result<T> = std::result::Result<T, PluginError>;
