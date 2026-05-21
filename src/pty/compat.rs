//! 100% 兼容 PTY 实现
//!
//! 特性：
//! - 使用 crossterm 切换到 raw mode，支持完整交互（方向键、TUI、单字符输入）
//! - 动态监听终端大小变化并 resize PTY
//! - stdin/stdout 完全透传，同时捕获到 ring buffer + 结构化事件
//! - 退出时自动恢复终端状态（即使 panic 也会尝试恢复）

use crossterm::terminal::{disable_raw_mode, enable_raw_mode, size as terminal_size};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde::Serialize;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

const RING_BUFFER_CAP: usize = 256 * 1024; // 256KB，足够大的回放缓冲

#[derive(Debug, Clone, Serialize)]
pub struct PtyEvent {
    #[serde(rename = "type")]
    pub kind: String,
    pub ts: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<usize>,
}

pub struct CompatPtySession {
    stdout: Arc<Mutex<Vec<u8>>>,
    stdin: Arc<Mutex<Vec<u8>>>,
    events: Arc<Mutex<Vec<PtyEvent>>>,
    ring_buffer: Arc<Mutex<Vec<u8>>>,
    child: Box<dyn portable_pty::Child + Send>,
    _raw_mode_guard: RawModeGuard,
}

/// 保证退出时恢复终端的 guard
struct RawModeGuard;

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}

impl CompatPtySession {
    /// 启动一个完全兼容的 PTY 会话
    pub fn spawn(
        command: &[String],
        cwd: &Path,
        extra_env: &HashMap<String, String>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        enable_raw_mode()?;
        let _raw_mode_guard = RawModeGuard;

        let pty_system = native_pty_system();
        let (cols, rows) = terminal_size().unwrap_or((120, 40));
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut cmd = CommandBuilder::new(&command[0]);
        cmd.args(&command[1..]);
        cmd.cwd(cwd);
        for (key, value) in extra_env {
            cmd.env(key, value);
        }

        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);

        let reader = pair.master.try_clone_reader()?;
        let mut writer = pair.master.take_writer()?;

        let stdout = Arc::new(Mutex::new(Vec::new()));
        let stdin = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(Mutex::new(Vec::new()));
        let ring = Arc::new(Mutex::new(Vec::new()));

        let stdout_clone = Arc::clone(&stdout);
        let events_clone = Arc::clone(&events);
        let ring_clone = Arc::clone(&ring);

        thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            let mut reader = reader;
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = &buffer[..n];

                        let _ = std::io::stdout().write_all(chunk);
                        let _ = std::io::stdout().flush();

                        stdout_clone.lock().unwrap().extend_from_slice(chunk);

                        let mut rb = ring_clone.lock().unwrap();
                        rb.extend_from_slice(chunk);
                        if rb.len() > RING_BUFFER_CAP {
                            let excess = rb.len() - RING_BUFFER_CAP;
                            rb.drain(0..excess);
                        }

                        let event = PtyEvent {
                            ts: chrono::Utc::now().to_rfc3339(),
                            kind: "PtyOutput".to_string(),
                            bytes: Some(n),
                        };
                        events_clone.lock().unwrap().push(event);
                    }
                    Err(_) => break,
                }
            }
        });

        let stdin_clone = Arc::clone(&stdin);
        thread::spawn(move || {
            let mut stdin_reader = std::io::stdin();
            let mut buffer = [0u8; 1024];
            loop {
                match stdin_reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        if writer.write_all(&buffer[..n]).is_err() {
                            break;
                        }
                        stdin_clone.lock().unwrap().extend_from_slice(&buffer[..n]);
                    }
                    Err(_) => break,
                }
            }
        });

        #[cfg(unix)]
        {
            let master = pair.master;
            thread::spawn(move || {
                use signal_hook::{consts::signal::SIGWINCH, iterator::Signals};
                if let Ok(mut signals) = Signals::new([SIGWINCH]) {
                    for _ in signals.forever() {
                        if let Ok((cols, rows)) = crossterm::terminal::size() {
                            let _ = master.resize(PtySize {
                                rows,
                                cols,
                                pixel_width: 0,
                                pixel_height: 0,
                            });
                        }
                    }
                }
            });
        }

        Ok(Self {
            stdout,
            stdin,
            events,
            ring_buffer: ring,
            child,
            _raw_mode_guard,
        })
    }

    /// 阻塞等待子进程结束，返回退出状态
    pub fn wait(&mut self) -> Result<portable_pty::ExitStatus, Box<dyn std::error::Error>> {
        Ok(self.child.wait()?)
    }

    /// 移出捕获数据，避免额外 clone
    pub fn take_captures(&self) -> (Vec<u8>, Vec<u8>, Vec<PtyEvent>, Vec<u8>) {
        (
            std::mem::take(&mut *self.stdout.lock().unwrap()),
            std::mem::take(&mut *self.stdin.lock().unwrap()),
            std::mem::take(&mut *self.events.lock().unwrap()),
            std::mem::take(&mut *self.ring_buffer.lock().unwrap()),
        )
    }
}
