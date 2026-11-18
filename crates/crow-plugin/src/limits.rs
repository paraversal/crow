//! One limits structure for package, registration, invocation and worker
//! budgets. Every bound the plan names lives here so a spike-driven adjustment
//! is visible in one place.

use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Limits {
    /// Manifest, entry and each declared asset.
    pub max_file_bytes: usize,
    /// Manifest + entry + declared assets.
    pub max_package_bytes: usize,
    pub max_registrations: usize,
    pub max_entries_per_call: usize,
    /// One display field (title, label, text, spoken value).
    pub max_display_bytes: usize,
    /// Serialized reply payload and the largest script string.
    pub max_result_bytes: usize,
    pub max_operations: u64,
    pub call_deadline: Duration,
    pub max_expression_depth: u32,
    pub max_call_levels: usize,
    pub max_array_size: usize,
    pub max_map_size: usize,
    /// A granted excerpt read through a request-local handle.
    pub max_read_bytes: usize,
    /// Explicit commands; full means busy.
    pub command_mailbox: usize,
    /// Refresh notifications; coalesced, a full lane reports busy.
    pub refresh_mailbox: usize,
    /// Pending handle submissions. One bound for the service; the default is
    /// the sum of the per-worker command and refresh lanes.
    pub service_mailbox: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_bytes: 256 * 1024,
            max_package_bytes: 2 * 1024 * 1024,
            max_registrations: 64,
            max_entries_per_call: 128,
            max_display_bytes: 4 * 1024,
            max_result_bytes: 64 * 1024,
            max_operations: 100_000,
            call_deadline: Duration::from_millis(50),
            max_expression_depth: 64,
            max_call_levels: 32,
            max_array_size: 1024,
            max_map_size: 1024,
            max_read_bytes: 64 * 1024,
            command_mailbox: 16,
            refresh_mailbox: 16,
            service_mailbox: 32,
        }
    }
}
