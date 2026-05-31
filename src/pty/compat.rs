//! 100% 兼容 PTY 实现
//!
//! 特性：
//! - 使用 crossterm 切换到 raw mode，支持完整交互（方向键、TUI、单字符输入）
//! - 动态监听终端大小变化并 resize PTY（Unix 上通过可响应 running 标志的 pending()+短睡眠轮询实现，支持干净 join，不再 forever 阻塞）
//! - stdin/stdout 完全透传，同时捕获到 ring buffer + 结构化事件
//! - 退出时自动 kill child + join 可安全退出的线程 + 恢复终端状态（即使 panic 也会尽力清理）

use crossterm::terminal::{disable_raw_mode, enable_raw_mode, size as terminal_size};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

const RING_BUFFER_CAP: usize = 256 * 1024; // 256KB，足够大的回放缓冲

pub type CaptureWriter = Box<dyn Write + Send>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyEvent {
    #[serde(rename = "type")]
    pub kind: String, // "PtyOutput" | "PtyInput"
    pub ts: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<usize>,
    /// Cumulative bytes written to this stream (stdout or stdin) after this chunk.
    /// Enables precise byte-range → timestamp correlation for `il dump timeline`.
    /// None for sessions recorded before this field was added.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
}

pub struct CompatPtySession {
    // 仅保留 ring (已按 256KB 循环覆盖) + events (结构化事件，通常不大)
    // stdout / stdin 原始字节流直接流式写入调用者提供的 sink，实现超长会话零内存增长
    events: Arc<Mutex<Vec<PtyEvent>>>,
    ring_buffer: Arc<Mutex<Vec<u8>>>,
    child: Box<dyn portable_pty::Child + Send>,
    _raw_mode_guard: RawModeGuard,
    // 线程生命周期管理（running 标志控制优雅退出，Acquire/Release 保证可见性）
    stdout_thread: Option<thread::JoinHandle<()>>,
    #[allow(dead_code)]
    stdin_thread: Option<thread::JoinHandle<()>>,
    resize_thread: Option<thread::JoinHandle<()>>,
    running: Arc<AtomicBool>,
}

/// 保证退出时恢复终端的 guard
struct RawModeGuard;

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}

impl CompatPtySession {
    /// 启动一个完全兼容的 PTY 会话（抽象 sink 流式捕获版）
    ///
    /// `stdout_capture` / `stdin_capture` 如果提供，会在捕获线程中直接 append 原始字节到 sink，
    /// 不再在内存中累积完整 stdout/stdin，从而支持数小时、数 GB 输出的超长 Agent 会话而不 OOM。
    /// ring buffer（256KB 循环）和 events 仍然保留在内存中，供 attach 和结构化事件使用。
    ///
    /// `live_stdout_feed` / `live_stdin_raw` 是可选的 live 钩子，用于增量结构化提取（conversation 等）。
    /// 它们在对应线程里被调用，接收原始 chunk，不应做重工作（由上层 tracker 负责）。
    pub fn spawn(
        command: &[String],
        cwd: &Path,
        extra_env: &HashMap<String, String>,
        stdout_capture: Option<CaptureWriter>,
        stdin_capture: Option<CaptureWriter>,
        live_stdout_feed: Option<super::LiveChunkFeed>,
        live_stdin_raw: Option<super::LiveChunkFeed>,
    ) -> Result<Self, anyhow::Error> {
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

        let events = Arc::new(Mutex::new(Vec::new()));
        let ring = Arc::new(Mutex::new(Vec::new()));

        let events_clone = Arc::clone(&events);
        let ring_clone = Arc::clone(&ring);

        let running = Arc::new(AtomicBool::new(true));
        let running_in: Arc<AtomicBool> = Arc::clone(&running);

        // 两个独立累积计数器：stdout 和 stdin 各自的字节偏移。
        // 用于 PtyEvent.offset，实现 stdin 原始字节区间 → 精确时间戳 的关联（timeline 核心）。
        let stdout_cumulative = Arc::new(AtomicU64::new(0));
        let stdin_cumulative = Arc::new(AtomicU64::new(0));

        // live 钩子需要克隆给各自线程（Arc 本身轻量）
        let live_stdout_feed = live_stdout_feed;
        let live_stdin_raw = live_stdin_raw;

        // stdout 捕获线程：实时写入 sink（如果提供）+ 更新 ring + 事件
        let mut stdout_writer = stdout_capture;
        let events_for_stdout = Arc::clone(&events_clone);
        let stdout_off = Arc::clone(&stdout_cumulative);
        let stdout_thread_handle = thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            let mut reader = reader;
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = &buffer[..n];

                        // 透传到用户终端
                        let _ = std::io::stdout().write_all(chunk);
                        let _ = std::io::stdout().flush();

                        // 流式写入调用者提供的原始 stdout sink（核心：不占内存）
                        if let Some(w) = &mut stdout_writer {
                            let _ = w.write_all(chunk);
                        }

                        // ring buffer（仅 256KB，常驻内存，用于 attach 尾部回放）
                        let mut rb = ring_clone.lock().unwrap_or_else(|p| p.into_inner());
                        rb.extend_from_slice(chunk);
                        if rb.len() > RING_BUFFER_CAP {
                            let excess = rb.len() - RING_BUFFER_CAP;
                            rb.drain(0..excess);
                        }

                        // 累积偏移 + 结构化事件（输出）
                        let new_offset =
                            stdout_off.fetch_add(n as u64, Ordering::Relaxed) + n as u64;
                        let event = PtyEvent {
                            ts: chrono::Utc::now().to_rfc3339(),
                            kind: "PtyOutput".to_string(),
                            bytes: Some(n),
                            offset: Some(new_offset),
                        };
                        events_for_stdout
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .push(event);

                        // live 结构化钩子（增量 feed 给 tracker，不阻塞主输出路径）
                        if let Some(feed) = &live_stdout_feed {
                            feed(chunk);
                        }
                    }
                    Err(_) => break,
                }
            }
            // 线程结束时 flush sink 写入
            if let Some(mut w) = stdout_writer {
                let _ = w.flush();
            }
        });

        // stdin 捕获线程：同样支持流式 sink + 小内存缓冲（用户输入通常很小）
        // 关键增强：同时产生带时间戳 + 累积偏移的 PtyInput 事件，用于后续时间轴对齐重建完整输入
        let mut stdin_writer = stdin_capture;
        let events_for_stdin = Arc::clone(&events);
        let live_stdin_raw_for_thread = live_stdin_raw;
        let stdin_off = Arc::clone(&stdin_cumulative);
        let stdin_thread_handle = thread::spawn(move || {
            let mut stdin_reader = std::io::stdin();
            let mut buffer = [0u8; 1024];
            let run = running_in;
            loop {
                if !run.load(Ordering::Acquire) {
                    break;
                }
                match stdin_reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        if writer.write_all(&buffer[..n]).is_err() {
                            break;
                        }
                        if let Some(w) = &mut stdin_writer {
                            let _ = w.write_all(&buffer[..n]);
                        }

                        // 累积偏移 + 记录带 ts 的输入事件（核心：让 timeline 重建有精确时间锚点）
                        let new_offset =
                            stdin_off.fetch_add(n as u64, Ordering::Relaxed) + n as u64;
                        let input_event = PtyEvent {
                            ts: chrono::Utc::now().to_rfc3339(),
                            kind: "PtyInput".to_string(),
                            bytes: Some(n),
                            offset: Some(new_offset),
                        };
                        events_for_stdin
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .push(input_event);

                        // live stdin 钩子：把原始用户输入字节交给 tracker 做实时 prompt 解析
                        if let Some(hook) = &live_stdin_raw_for_thread {
                            hook(&buffer[..n]);
                        }
                    }
                    Err(_) => break,
                }
            }
            if let Some(mut w) = stdin_writer {
                let _ = w.flush();
            }
        });

        let resize_thread: Option<thread::JoinHandle<()>>;
        #[cfg(unix)]
        {
            let master = pair.master;
            let run = Arc::clone(&running);
            resize_thread = Some(thread::spawn(move || {
                use signal_hook::{consts::signal::SIGWINCH, iterator::Signals};
                if let Ok(mut signals) = Signals::new([SIGWINCH]) {
                    loop {
                        if !run.load(Ordering::Acquire) {
                            break;
                        }
                        for _ in signals.pending() {
                            if let Ok((cols, rows)) = crossterm::terminal::size() {
                                let _ = master.resize(PtySize {
                                    rows,
                                    cols,
                                    pixel_width: 0,
                                    pixel_height: 0,
                                });
                            }
                        }
                        if !run.load(Ordering::Acquire) {
                            break;
                        }
                        thread::sleep(std::time::Duration::from_millis(80));
                    }
                }
            }));
        }
        #[cfg(not(unix))]
        {
            let _ = pair.master;
            resize_thread = None;
        }

        Ok(Self {
            events,
            ring_buffer: ring,
            child,
            _raw_mode_guard,
            stdout_thread: Some(stdout_thread_handle),
            stdin_thread: Some(stdin_thread_handle),
            resize_thread,
            running,
        })
    }

    /// 阻塞等待子进程结束，返回退出状态
    /// 子进程退出后再通知捕获线程停止，确保尽量读完 PTY 尾部输出。
    pub fn wait(&mut self) -> Result<portable_pty::ExitStatus, anyhow::Error> {
        let status = self.child.wait()?;
        self.running.store(false, Ordering::Release);
        // 等待 stdout 捕获线程完成（保证最后输出被读完）
        if let Some(h) = self.stdout_thread.take() {
            let _ = h.join();
        }
        // resize 监听线程使用 pending()+短睡眠，可在 80ms 内响应 shutdown，安全 join
        if let Some(h) = self.resize_thread.take() {
            let _ = h.join();
        }
        // stdin 线程仍可能阻塞在用户终端 read()，进程退出时由 OS 回收（已知设计权衡）
        Ok(status)
    }

    /// 移出捕获数据（仅 events + ring，原始 stdout/stdin 已在外部 sink 中）
    pub fn take_captures(&self) -> (Vec<PtyEvent>, Vec<u8>) {
        (
            std::mem::take(&mut *self.events.lock().unwrap_or_else(|p| p.into_inner())),
            std::mem::take(&mut *self.ring_buffer.lock().unwrap_or_else(|p| p.into_inner())),
        )
    }
}

impl Drop for CompatPtySession {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        let _ = self.child.kill();
        if let Some(h) = self.stdout_thread.take() {
            let _ = h.join();
        }
        // resize 线程已改为 pending()+短睡眠轮询，可安全 join（最多等 ~80ms）
        if let Some(h) = self.resize_thread.take() {
            let _ = h.join();
        }
        // stdin 线程读取用户终端，难以可靠唤醒；进程退出时 OS 自动回收（已知设计权衡）
    }
}

// 旧的内存警告静态量已移除，改为磁盘流式捕获后不再需要。
