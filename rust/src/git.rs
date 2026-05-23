//! Git operations: run commands, get changed files, show file content
//!
//! Uses std::process::Command (blocking). Wrap in spawn_blocking if needed.

use anyhow::{Context, Result, bail};
use std::path::Path;

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

/// A changed file from git diff
#[derive(Debug, Clone)]
pub struct ChangedFile {
    pub path: String,
    pub status: FileStatus,
}

/// Git file status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
}

/// Get list of changed files between two refs
pub fn get_changed_files(base_ref: &str, head_ref: &str, cwd: &Path) -> Result<Vec<ChangedFile>> {
    let output = run_git(&["diff", "--name-status", base_ref, head_ref], cwd)?;

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
        let status = match status_char {
            'A' => FileStatus::Added,
            'D' => FileStatus::Deleted,
            'R' => FileStatus::Renamed,
            _ => FileStatus::Modified,
        };

        // For renamed files, use the new name (second path)
        let path = if status == FileStatus::Renamed && parts.len() >= 3 {
            parts[2].to_string()
        } else if parts.len() >= 2 {
            parts[1].to_string()
        } else {
            continue;
        };

        files.push(ChangedFile { path, status });
    }

    Ok(files)
}

/// Get diff for a specific file
pub fn get_file_diff(
    base_ref: &str,
    head_ref: &str,
    file_path: &str,
    cwd: &Path,
) -> Result<String> {
    run_git(&["diff", base_ref, head_ref, "--", file_path], cwd)
}

/// Get file content at a specific ref
pub fn get_file_at_ref(ref_: &str, file_path: &str, cwd: &Path) -> Option<String> {
    let spec = format!("{}:{}", ref_, file_path);
    run_git_optional(&["show", &spec], cwd)
}

/// Count total lines in a file (for diff annotation width calculation)
pub fn count_file_lines(content: &str) -> usize {
    if content.is_empty() {
        0
    } else {
        content.lines().count()
    }
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

    #[test]
    fn test_count_file_lines() {
        assert_eq!(count_file_lines(""), 0);
        assert_eq!(count_file_lines("one"), 1);
        assert_eq!(count_file_lines("one\ntwo"), 2);
        assert_eq!(count_file_lines("one\ntwo\n"), 2);
    }
}
