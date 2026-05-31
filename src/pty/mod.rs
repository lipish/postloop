pub mod compat;
pub mod content_filter;
pub mod terminal_input;
pub mod vt100_recorder;

pub use compat::{CaptureWriter, CompatPtySession, PtyEvent};
pub use terminal_input::{extract_raw_submissions, RawSubmission};
pub use vt100_recorder::file_has_ansi;

/// 给 live 增量 tracker 用的 chunk feed 回调类型（stdout 原始字节或 stdin 原始字节）。
/// 避免在 spawn 签名和调用处重复写极长的 dyn Fn 复杂类型，满足 clippy::type-complexity。
pub type LiveChunkFeed = std::sync::Arc<dyn Fn(&[u8]) + Send + Sync>;
