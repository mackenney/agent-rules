//! Integration tests — CLI behavior without LLM calls.
//!
//! Run: cargo nextest run --test integration

#[path = "../common/mod.rs"]
mod common;

mod cache;
mod check;
mod rules;
