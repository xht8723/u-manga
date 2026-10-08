//! Closed serialized states at the Jobs boundary; existing wire values are unchanged.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobOrigin {
    Reader,
    Batch,
    Explicit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    Revision,
    Configuration,
    Requirements,
    Processing,
    Storage,
}

#[derive(Debug)]
pub struct ExecutionError {
    pub kind: FailureKind,
    pub message: String,
}
impl std::fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for ExecutionError {}
impl ExecutionError {
    pub fn error(kind: FailureKind, message: impl Into<String>) -> anyhow::Error {
        Self {
            kind,
            message: message.into(),
        }
        .into()
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    Paused,
    Failed,
    Cancelled,
    Complete,
}
impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Complete => "complete",
        }
    }
}
impl PartialEq<&str> for JobStatus {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}
impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
// Only compile-time literals enter this compatibility constructor. Persisted data uses
// serde's closed enum validation, never this assertion.
impl From<&'static str> for JobStatus {
    fn from(value: &'static str) -> Self {
        match value {
            "queued" => Self::Queued,
            "running" => Self::Running,
            "paused" => Self::Paused,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "complete" => Self::Complete,
            _ => panic!("Invalid internal job status"),
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobAction {
    Pause,
    Resume,
    Retry,
    Cancel,
}
impl JobAction {
    pub fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "pause" => Ok(Self::Pause),
            "resume" => Ok(Self::Resume),
            "retry" => Ok(Self::Retry),
            "cancel" => Ok(Self::Cancel),
            _ => anyhow::bail!("Unknown job action"),
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Retry => "retry",
            Self::Cancel => "cancel",
        }
    }
}
