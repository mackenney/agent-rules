//! Core data types: Rule, Verdict, FileVerdict, PRReport, FileDiff

use serde::{Deserialize, Serialize};

/// Placeholder - will be implemented in step-02
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
}
