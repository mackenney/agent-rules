//! Git operations: run commands, get changed files, show file content
//!
//! Uses std::process::Command (blocking). Wrap in spawn_blocking if needed.

use anyhow::{bail, Context, Result};
use std::path::Path;

use crate::schema::FileDiff;

/// List of binary file extensions to skip
const BINARY_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "bmp", "ico", "webp", "svg", "pdf", "doc", "docx", "xls", "xlsx",
    "ppt", "pptx", "zip", "tar", "gz", "bz2", "7z", "rar", "exe", "dll", "so", "dylib", "bin",
    "ttf", "otf", "woff", "woff2", "eot", "mp3", "mp4", "avi", "mov", "mkv", "webm", "lock",
    "lockb",
];

/// Run a git command and return stdout
pub fn run_git(args: &[&str], cwd: &Path) -> Result<String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .with_context(|| format!("failed to run: git {}", args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git {} failed: {}", args.join(" "), stderr.trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Run git command, returning None on failure instead of error
pub fn run_git_optional(args: &[&str], cwd: &Path) -> Option<String> {
    run_git(args, cwd).ok()
}

/// Get the repository root directory
pub fn get_repo_root(cwd: &Path) -> Result<std::path::PathBuf> {
    let output = run_git(&["rev-parse", "--show-toplevel"], cwd)?;
    Ok(std::path::PathBuf::from(output.trim()))
}

/// Check if a file path has a binary extension
pub fn is_binary_extension(path: &str) -> bool {
    path.rsplit('.')
        .next()
        .map(|ext| BINARY_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Get list of changed files between two refs, enriched with diff and content
pub fn get_changed_files(
    base_ref: &str,
    head_ref: &str,
    repo_root: &Path,
    max_file_bytes: u64,
) -> Result<Vec<FileDiff>> {
    let output = run_git(&["diff", "--name-status", base_ref, head_ref], repo_root)?;

    let mut files = Vec::new();
    for line in output.lines() {
        if line.trim().is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split('\t').collect();
        if parts.is_empty() {
            continue;
        }

        let status_char = parts[0].chars().next().unwrap_or('M');
        let is_deleted = status_char == 'D';
        let is_new = status_char == 'A';
        let is_renamed = status_char == 'R';

        // For renamed files, use the new name (second path)
        let path = if is_renamed && parts.len() >= 3 {
            parts[2].to_string()
        } else if parts.len() >= 2 {
            parts[1].to_string()
        } else {
            continue;
        };

        let diff =
            run_git(&["diff", base_ref, head_ref, "--", &path], repo_root).unwrap_or_default();

        let is_binary = is_binary_extension(&path) || diff.contains("Binary files");

        let (content, is_oversized, oversized_bytes) = if !is_deleted && !is_binary {
            let spec = format!("{}:{}", head_ref, path);
            match run_git_optional(&["show", &spec], repo_root) {
                Some(c) => {
                    let byte_len = c.len() as u64;
                    if byte_len > max_file_bytes {
                        (None, true, Some(byte_len))
                    } else {
                        (Some(c), false, None)
                    }
                }
                None => (None, false, None),
            }
        } else {
            (None, false, None)
        };

        files.push(FileDiff {
            path,
            diff,
            content,
            is_binary,
            is_deleted,
            is_new,
            is_oversized,
            oversized_bytes,
        });
    }

    Ok(files)
}

/// Read a local file, returning None if it exceeds max_bytes or is not valid UTF-8
pub fn get_file_content(path: &Path, max_bytes: u64) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() as u64 > max_bytes {
        return None;
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_binary_extension() {
        assert!(is_binary_extension("image.png"));
        assert!(is_binary_extension("FILE.PNG"));
        assert!(is_binary_extension("path/to/doc.pdf"));
        assert!(!is_binary_extension("code.rs"));
        assert!(!is_binary_extension("Makefile"));
    }
}
