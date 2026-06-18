//! End-to-end tests — real Anthropic API calls against test-repo.
//!
//! Requires ANTHROPIC_API_KEY (Anthropic tests) and/or OPENROUTER_API_KEY
//! (OpenRouter tests). Tests skip individually when their required key is absent.
//!
//! Run: cargo nextest run --test e2e --features test-e2e

#[path = "../common/mod.rs"]
mod common;

mod check;
