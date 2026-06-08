//! Handler for `agent-rules cache`

use anyhow::Result;
use owo_colors::OwoColorize;

use agent_rules::cache::{Cache, CacheManager};
use agent_rules::git::get_repo_root;
use agent_rules::reporter::Stylesheet;

use crate::CacheCommands;

pub fn run_cache(command: CacheCommands, colors: &Stylesheet) -> Result<i32> {
    match command {
        CacheCommands::Stats { repo } => {
            let repo_root = match repo {
                Some(r) => r,
                None => get_repo_root(&std::env::current_dir()?)?,
            };
            let cache = CacheManager::new(&repo_root)?;
            let stats = cache.stats()?;

            println!("{}", "Cache Statistics".bold());
            println!("  Entries: {}", stats.total_entries);
            println!("  Size: {} KB", stats.total_size_bytes / 1024);
            println!("  Total hits: {}", stats.total_hits);

            if let Some(oldest) = stats.oldest_entry {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs_f64();
                let age_secs = (now - oldest).max(0.0);
                let age_str = if age_secs < 3600.0 {
                    format!("{:.0}m ago", age_secs / 60.0)
                } else if age_secs < 86400.0 {
                    format!("{:.1}h ago", age_secs / 3600.0)
                } else {
                    format!("{:.1}d ago", age_secs / 86400.0)
                };
                println!("  Oldest entry: {}", age_str);
            }

            Ok(0)
        }
        CacheCommands::Clear { repo, yes } => {
            let repo_root = match repo {
                Some(r) => r,
                None => get_repo_root(&std::env::current_dir()?)?,
            };
            if !yes {
                eprint!("Clear all cache entries? [y/N] ");
                std::io::Write::flush(&mut std::io::stderr())?;

                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;

                if !input.trim().eq_ignore_ascii_case("y") {
                    println!("Cancelled.");
                    return Ok(0);
                }
            }

            let cache = CacheManager::new(&repo_root)?;
            let count = cache.clear()?;

            println!(
                "{} Cleared {} cache entries",
                "✓".style(colors.success),
                count
            );

            Ok(0)
        }
    }
}
