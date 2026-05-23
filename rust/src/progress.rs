//! Progress reporting: TTY progress bar, CI output

/// Placeholder - will be implemented in step-07
pub trait ProgressReporter: Send + Sync {
    fn set_total(&self, n: usize);
    fn on_file_start(&self, path: &str);
    fn on_file_done(&self, path: &str);
    fn finish(&self);
}
