//! Progress reporting: TTY progress bar and null reporter
//!
//! Uses indicatif for TTY progress bars with spinner.
//! TTY detection uses std::io::IsTerminal (stable since Rust 1.70).

use indicatif::{ProgressBar, ProgressStyle};
use std::io::IsTerminal;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// Progress reporter trait.
///
/// Methods are per-rule-call (one call = one rule × one file), not per-file.
#[allow(dead_code)] // add_total and log reserved for dynamic total updates and trace logging
pub trait ProgressReporter: Send + Sync {
    fn set_total(&self, n: usize);
    fn add_total(&self, n: usize);
    fn on_call_start(&self, label: &str);
    fn on_call_done(&self, label: &str);
    fn log(&self, msg: &str);
    fn finish(&self);
}

/// TTY progress bar using indicatif
pub struct TtyProgress {
    bar: Mutex<ProgressBar>,
    completed: AtomicUsize,
    total: AtomicUsize,
}

impl TtyProgress {
    pub fn new(total: usize) -> Self {
        let bar = ProgressBar::new(total as u64);
        bar.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.cyan} [{bar:40.cyan/dim}] {pos}/{len}  {msg}")
                .expect("invalid template")
                .progress_chars("█░░"),
        );
        bar.enable_steady_tick(std::time::Duration::from_millis(80));

        Self {
            bar: Mutex::new(bar),
            completed: AtomicUsize::new(0),
            total: AtomicUsize::new(total),
        }
    }
}

impl Default for TtyProgress {
    fn default() -> Self {
        Self::new(0)
    }
}

impl ProgressReporter for TtyProgress {
    fn set_total(&self, n: usize) {
        self.total.store(n, Ordering::SeqCst);
        if let Ok(bar) = self.bar.lock() {
            bar.set_length(n as u64);
        }
    }

    fn add_total(&self, n: usize) {
        let prev = self.total.fetch_add(n, Ordering::SeqCst);
        if let Ok(bar) = self.bar.lock() {
            bar.set_length((prev + n) as u64);
        }
    }

    fn on_call_start(&self, label: &str) {
        if let Ok(bar) = self.bar.lock() {
            let display = if label.len() > 50 {
                format!("...{}", &label[label.len() - 47..])
            } else {
                label.to_string()
            };
            bar.set_message(display);
        }
    }

    fn on_call_done(&self, _label: &str) {
        let completed = self.completed.fetch_add(1, Ordering::SeqCst) + 1;
        if let Ok(bar) = self.bar.lock() {
            bar.set_position(completed as u64);
        }
    }

    fn log(&self, msg: &str) {
        if let Ok(bar) = self.bar.lock() {
            bar.println(msg);
        }
    }

    fn finish(&self) {
        if let Ok(bar) = self.bar.lock() {
            bar.finish_and_clear();
        }
    }
}

/// Null progress reporter — no output
pub struct NullProgress;

impl ProgressReporter for NullProgress {
    fn set_total(&self, _n: usize) {}
    fn add_total(&self, _n: usize) {}
    fn on_call_start(&self, _label: &str) {}
    fn on_call_done(&self, _label: &str) {}
    fn log(&self, _msg: &str) {}
    fn finish(&self) {}
}

/// Create appropriate progress reporter for the current environment.
///
/// Returns a TTY progress bar when stderr is a terminal and not in CI;
/// returns NullProgress otherwise.
pub fn create_progress_reporter(total: usize) -> Box<dyn ProgressReporter> {
    let is_tty = std::io::stderr().is_terminal();
    let is_ci = std::env::var("CI").is_ok() || std::env::var("GITHUB_ACTIONS").is_ok();
    if is_tty && !is_ci {
        Box::new(TtyProgress::new(total))
    } else {
        Box::new(NullProgress)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_null_no_panic() {
        let p = NullProgress;
        p.set_total(10);
        p.add_total(5);
        p.on_call_start("src/api/auth.ts[auth/no-hardcoded-secrets]");
        p.on_call_done("src/api/auth.ts[auth/no-hardcoded-secrets]");
        p.log("message");
        p.finish();
    }

    #[test]
    fn progress_tty_counting() {
        let p = TtyProgress::new(3);
        p.set_total(3);
        assert_eq!(p.total.load(Ordering::SeqCst), 3);

        p.on_call_done("a.rs[rule]");
        assert_eq!(p.completed.load(Ordering::SeqCst), 1);

        p.on_call_done("b.rs[rule]");
        assert_eq!(p.completed.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn progress_add_total() {
        let p = TtyProgress::new(3);
        p.add_total(2);
        assert_eq!(p.total.load(Ordering::SeqCst), 5);
    }

    #[test]
    fn progress_reporter_factory_returns_box() {
        let _reporter = create_progress_reporter(10);
    }
}
