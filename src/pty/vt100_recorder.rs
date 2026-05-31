//! 将 PTY stdout 字节流喂入 vt100 虚拟终端，提取稳定屏幕快照。

use serde::Serialize;

use crate::pty::content_filter::{filter_content_lines, is_noise_line};

use vt100::Parser;

#[derive(Debug, Clone, Serialize)]
pub struct ScreenSnapshot {
    pub seq: usize,
    #[serde(rename = "screen")]
    pub contents: String,
}

pub struct Vt100Recorder {
    parser: Parser,
    snapshots: Vec<ScreenSnapshot>,
    last_contents: String,
}

impl Vt100Recorder {
    pub fn new(rows: u16, cols: u16) -> Self {
        Self {
            parser: Parser::new(rows, cols, 1000),
            snapshots: Vec::new(),
            last_contents: String::new(),
        }
    }

    /// 按 PTY 捕获顺序回放字节流，仅在屏幕内容变化时记录快照。
    pub fn replay(mut self, stdout: &[u8]) -> Vec<ScreenSnapshot> {
        self.feed(stdout);
        self.snapshots
    }

    /// 增量喂入字节（支持 live 捕获期间持续快照），避免退出时全量重放。
    /// 每块处理后立即尝试 maybe_snapshot。
    pub fn feed(&mut self, data: &[u8]) {
        for chunk in data.chunks(8192) {
            self.parser.process(chunk);
            self.maybe_snapshot();
        }
        self.maybe_snapshot();
    }

    /// 流式版本：从 reader 读取，避免全量载入大文件到内存。
    pub fn replay_from_reader<R: std::io::Read>(mut self, mut reader: R) -> Vec<ScreenSnapshot> {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    self.parser.process(&buf[..n]);
                    self.maybe_snapshot();
                }
                Err(_) => break,
            }
        }
        self.maybe_snapshot();
        self.snapshots
    }

    /// 返回当前已积累的快照切片（live 模式下持续增长）。
    pub fn snapshots(&self) -> &[ScreenSnapshot] {
        &self.snapshots
    }

    /// 强制尝试一次快照（通常 feed 内部已调用）。
    pub fn force_snapshot(&mut self) {
        self.maybe_snapshot();
    }

    pub(crate) fn maybe_snapshot(&mut self) {
        let contents = normalize_screen(&self.parser.screen().contents());
        if contents == self.last_contents {
            return;
        }
        self.last_contents = contents.clone();
        self.snapshots.push(ScreenSnapshot {
            seq: self.snapshots.len(),
            contents,
        });
    }
}

pub fn normalize_screen(text: &str) -> String {
    text.lines()
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 提取 after 相对 before 新增的非空行（vt100 屏幕 diff）。
pub fn lines_added(before: &str, after: &str) -> Vec<String> {
    let before_set: std::collections::HashSet<&str> = before
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();

    filter_content_lines(
        after
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !before_set.contains(line) && !is_noise_line(line))
            .map(str::to_string)
            .collect(),
    )
}

/// 无 ANSI 的旧日志：按行去重提取正文（best-effort 回退）。
pub fn unique_content_lines(text: &str) -> Vec<String> {
    filter_content_lines(
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !is_noise_line(l))
            .map(str::to_string)
            .collect(),
    )
}

pub fn stdout_has_ansi(stdout: &[u8]) -> bool {
    stdout.contains(&0x1b)
}

/// 仅 peek 文件前缀判断是否包含 ANSI 转义（避免大文件全量载入内存）。
pub fn file_has_ansi(path: &std::path::Path) -> bool {
    use std::io::Read;
    if let Ok(mut f) = std::fs::File::open(path) {
        let mut buf = [0u8; 8192];
        if let Ok(n) = f.read(&mut buf) {
            return buf[..n].contains(&0x1b);
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_dedupes_identical_screens() {
        let mut recorder = Vt100Recorder::new(24, 80);
        recorder.parser.process(b"Hello\r\nWorld\r\n");
        recorder.maybe_snapshot();
        recorder.maybe_snapshot();
        assert_eq!(recorder.snapshots.len(), 1);
    }

    #[test]
    fn file_has_ansi_peeks_prefix_without_loading_whole_file() {
        use std::io::Write;
        use std::path::Path;
        use tempfile::NamedTempFile;

        // Contains ESC in first 8 KiB → true
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(b"header\x1b[32m green text \x1b[0m\n").unwrap();
        f.flush().unwrap();
        assert!(file_has_ansi(f.path()));

        // No ESC anywhere
        let mut f2 = NamedTempFile::new().unwrap();
        f2.write_all(b"completely clean output\n123\n").unwrap();
        f2.flush().unwrap();
        assert!(!file_has_ansi(f2.path()));

        // Missing file is treated as "no ANSI"
        assert!(!file_has_ansi(Path::new(
            "/tmp/does-not-exist-ansi-test-928374"
        )));
    }
}
