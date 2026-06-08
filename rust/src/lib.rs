#![deny(missing_docs)]
//! agent-rules — directory-scoped LLM rule enforcement for PR reviews.
//!
//! Evaluates changed files against rules defined in `.agent-rules.toml` files
//! using an LLM as the evaluator. Rules cascade from the repo root to
//! subdirectories; child rules override parent rules by ID.

pub mod agentic;
pub mod cache;
pub mod config;
pub mod evaluator;
pub mod git;
pub mod llm;
pub mod openrouter;
pub mod parser;
pub mod progress;
pub mod prompt;
pub mod reporter;
pub mod resolver;
pub mod runner;
pub mod schema;
