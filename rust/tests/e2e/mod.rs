//! End-to-end tests — real Anthropic API calls against test-repo.
//!
//! Requires: ANTHROPIC_API_KEY environment variable.
//! Tests skip individually when the key is absent.
//!
//! Run: cargo nextest run --test e2e --features test-e2e

#[path = "../common/mod.rs"]
mod common;

mod check;
