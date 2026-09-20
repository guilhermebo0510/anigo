//! P2: Lock-free in-process metrics for the anigo-mcp server.
//!
//! Counters are stored as `AtomicU64` so incrementing them is wait-free on
//! x86-64. A `snapshot()` method produces a JSON value suitable for returning
//! from an `anigo_get_metrics` tool or for embedding in logs / support
//! bundles. We do not (yet) expose a Prometheus scrape endpoint — a future
//! iteration can add an HTTP admin listener when needed.

use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub struct McpMetrics {
    pub start_instant: Instant,
    pub start_epoch_secs: u64,
    pub tool_calls_total: AtomicU64,
    pub tool_calls_success: AtomicU64,
    pub tool_calls_error: AtomicU64,
    pub bridge_commands_sent: AtomicU64,
    pub bridge_commands_failed: AtomicU64,
    pub frames_rendered: AtomicU64,
    pub bytes_written_to_fs: AtomicU64,
    pub fs_sandbox_rejections: AtomicU64,
    pub json_rpc_requests: AtomicU64,
    pub json_rpc_parse_errors: AtomicU64,
}

impl McpMetrics {
    pub fn new() -> Self {
        Self {
            start_instant: Instant::now(),
            start_epoch_secs: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            tool_calls_total: AtomicU64::new(0),
            tool_calls_success: AtomicU64::new(0),
            tool_calls_error: AtomicU64::new(0),
            bridge_commands_sent: AtomicU64::new(0),
            bridge_commands_failed: AtomicU64::new(0),
            frames_rendered: AtomicU64::new(0),
            bytes_written_to_fs: AtomicU64::new(0),
            fs_sandbox_rejections: AtomicU64::new(0),
            json_rpc_requests: AtomicU64::new(0),
            json_rpc_parse_errors: AtomicU64::new(0),
        }
    }

    pub fn snapshot(&self) -> Value {
        let uptime = self.start_instant.elapsed().as_secs_f64();
        let total = self.tool_calls_total.load(Relaxed);
        let success = self.tool_calls_success.load(Relaxed);
        let errors = self.tool_calls_error.load(Relaxed);
        json!({
            "uptime_seconds": uptime,
            "started_at_epoch_secs": self.start_epoch_secs,
            "tool_calls_total": total,
            "tool_calls_success": success,
            "tool_calls_error": errors,
            "tool_success_rate": if total > 0 { (success as f64) / (total as f64) } else { 1.0 },
            "bridge_commands_sent": self.bridge_commands_sent.load(Relaxed),
            "bridge_commands_failed": self.bridge_commands_failed.load(Relaxed),
            "frames_rendered": self.frames_rendered.load(Relaxed),
            "bytes_written_to_fs": self.bytes_written_to_fs.load(Relaxed),
            "fs_sandbox_rejections": self.fs_sandbox_rejections.load(Relaxed),
            "json_rpc_requests": self.json_rpc_requests.load(Relaxed),
            "json_rpc_parse_errors": self.json_rpc_parse_errors.load(Relaxed),
            "tools_per_second_avg": if uptime > 0.0 { (total as f64) / uptime } else { 0.0 },
        })
    }
}

impl Default for McpMetrics {
    fn default() -> Self { Self::new() }
}
