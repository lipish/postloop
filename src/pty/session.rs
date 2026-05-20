use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;

const RING_BUFFER_CAP: usize = 64 * 1024;

pub struct PtySession {
    pub stdout: Arc<Mutex<Vec<u8>>>,
    pub stdin: Arc<Mutex<Vec<u8>>>,
    pub events: Arc<Mutex<Vec<String>>>,
    ring_buffer: Arc<Mutex<Vec<u8>>>,
}

impl PtySession {
    pub fn spawn(command: &[String], repo_root: &std::path::Path, shell_setup: Option<&str>) -> Result<Self, Box<dyn std::error::Error>> {
        // shell_setup 可在此处注入前缀命令，例如 source venv && ...
        if let Some(setup) = shell_setup {
            println!("Shell setup: {}", setup);
        }
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows: 40,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut cmd = CommandBuilder::new(&command[0]);
        cmd.args(&command[1..]);
        cmd.cwd(repo_root);

        let mut child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);

        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        let stdout = Arc::new(Mutex::new(Vec::new()));
        let stdin = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(Mutex::new(Vec::new()));
        let ring = Arc::new(Mutex::new(Vec::new()));

        let stdout_clone = Arc::clone(&stdout);
        let events_clone = Arc::clone(&events);
        let ring_clone = Arc::clone(&ring);

        thread::spawn(move || {
            let mut buffer = [0u8; 4096];
            let mut reader = reader;
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = &buffer[..n];
                        stdout_clone.lock().unwrap().extend_from_slice(chunk);
                        ring_clone.lock().unwrap().extend_from_slice(chunk);
                        if ring_clone.lock().unwrap().len() > RING_BUFFER_CAP {
                            let excess = ring_clone.lock().unwrap().len() - RING_BUFFER_CAP;
                            ring_clone.lock().unwrap().drain(0..excess);
                        }
                        let event = format!(r#"{{"type":"PtyOutput","bytes":{}}}"#, n);
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
            let mut writer = writer;
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

        Ok(Self {
            stdout,
            stdin,
            events,
            ring_buffer: ring,
        })
    }

    pub fn get_ring_buffer(&self) -> Vec<u8> {
        self.ring_buffer.lock().unwrap().clone()
    }
}
