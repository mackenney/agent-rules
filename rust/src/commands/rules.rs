//! Handler for `agent-rules rules`

use anyhow::Result;
use owo_colors::OwoColorize;

use agent_rules::parser::{RULE_FILE_NAME, parse_rule_file, validate_rule};
use agent_rules::reporter::Stylesheet;
use agent_rules::resolver::{find_all_rule_files, resolve_rules_for_file};
use agent_rules::schema::Severity;

use crate::RulesCommands;

pub fn run_rules(command: RulesCommands, colors: &Stylesheet) -> Result<i32> {
    match command {
        RulesCommands::List { path, repo } => {
            let repo_root = match repo {
                Some(r) => r,
                None => std::env::current_dir()?,
            };

            let abs_path = if path.is_absolute() {
                path.clone()
            } else {
                repo_root.join(&path)
            };
            let rules = resolve_rules_for_file(&abs_path, &repo_root)?;

            if rules.is_empty() {
                println!("No rules apply to {}", path.display());
                return Ok(0);
            }

            println!("{} rules apply to {}:", rules.len().bold(), path.display());
            println!();

            for rule in &rules {
                let severity_str = match rule.severity {
                    Severity::Error => "error".style(colors.error),
                    Severity::Warn => "warn".style(colors.warning),
                };

                println!(
                    "  {} {} [{}]",
                    "•".style(colors.note),
                    rule.name.bold(),
                    rule.id.style(colors.dim),
                );
                println!("    Severity: {}", severity_str);
                if !rule.glob_include.is_empty() && rule.glob_include != vec!["**/*"] {
                    println!("    Include: {}", rule.glob_include.join(", "));
                }
                if !rule.glob_exclude.is_empty() {
                    println!("    Exclude: {}", rule.glob_exclude.join(", "));
                }
                println!();
            }

            Ok(0)
        }
        RulesCommands::Validate { repo } => {
            let repo_root = match repo {
                Some(r) => r,
                None => std::env::current_dir()?,
            };

            let rule_files = find_all_rule_files(&repo_root)?;

            if rule_files.is_empty() {
                println!(
                    "No {} files found in {}",
                    RULE_FILE_NAME,
                    repo_root.display()
                );
                return Ok(0);
            }

            println!(
                "Validating {} rule files in {}",
                rule_files.len(),
                repo_root.display()
            );
            println!();

            let mut all_valid = true;
            let mut total_rules = 0usize;
            let mut all_rule_ids: std::collections::HashMap<String, Vec<String>> =
                std::collections::HashMap::new();

            for path in &rule_files {
                let relative = path
                    .strip_prefix(&repo_root)
                    .unwrap_or(path)
                    .display()
                    .to_string();

                match parse_rule_file(path) {
                    Ok(rf) => {
                        let mut file_errors = Vec::new();

                        for rule in &rf.rules {
                            let errors = validate_rule(rule);
                            file_errors.extend(errors);

                            all_rule_ids
                                .entry(rule.id.clone())
                                .or_default()
                                .push(relative.clone());

                            total_rules += 1;
                        }

                        if file_errors.is_empty() {
                            println!(
                                "  {} {}  {} rule(s)",
                                "✓".style(colors.success),
                                relative,
                                rf.rules.len()
                            );
                        } else {
                            all_valid = false;
                            println!("  {} {}", "✗".style(colors.error), relative);
                            for err in file_errors {
                                println!("    - {}", err.style(colors.error));
                            }
                        }
                    }
                    Err(e) => {
                        all_valid = false;
                        println!("  {} {}", "✗".style(colors.error), relative);
                        println!("    - {}", e.to_string().style(colors.error));
                    }
                }
            }

            // Only report conflicts for files that are NOT in ancestor-descendant relationship.
            // Parent-child overrides (same ID in parent dir and child dir) are valid cascade.
            let conflicts: Vec<_> = all_rule_ids
                .iter()
                .filter(|(_, files)| {
                    if files.len() < 2 {
                        return false;
                    }
                    // Check if any pair of files is unrelated (not ancestor-descendant)
                    let dirs: Vec<std::path::PathBuf> = files
                        .iter()
                        .map(|f| {
                            std::path::Path::new(f)
                                .parent()
                                .map(|p| p.to_path_buf())
                                .unwrap_or_default()
                        })
                        .collect();
                    for i in 0..dirs.len() {
                        for j in (i + 1)..dirs.len() {
                            if !dirs[i].starts_with(&dirs[j]) && !dirs[j].starts_with(&dirs[i]) {
                                return true;
                            }
                        }
                    }
                    false
                })
                .collect();

            if !conflicts.is_empty() {
                all_valid = false;
                println!();
                println!(
                    "{} Cross-file rule ID conflicts detected:",
                    "Error:".style(colors.error)
                );
                for (id, files) in &conflicts {
                    println!("  {} defined in:", id.style(colors.note));
                    for f in *files {
                        println!("    - {}", f);
                    }
                }
            }

            println!();
            println!(
                "Validated {} file(s), {} rule(s) total.",
                rule_files.len(),
                total_rules
            );

            if all_valid {
                println!(
                    "{} All {} rules in {} files are valid",
                    "✓".style(colors.success),
                    total_rules,
                    rule_files.len()
                );
                Ok(0)
            } else {
                println!("{} Validation found errors", "✗".style(colors.error));
                // Exit 1 for rule validation failures (not config errors — rules validate is a linting command)
                Ok(1)
            }
        }
    }
}
