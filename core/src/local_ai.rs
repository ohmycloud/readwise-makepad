use serde::{Deserialize, Serialize};

pub const LOCAL_MODEL_ALIAS: &str = "readwise-local";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct LocalAiSettings {
    pub binary_path: String,
    pub model_path: String,
    pub port: u16,
    pub context_size: u32,
    pub threads: u16,
    pub gpu_layers: u16,
    pub auto_start: bool,
}

impl Default for LocalAiSettings {
    fn default() -> Self {
        Self {
            binary_path: "llama-server".to_string(),
            model_path: String::new(),
            port: 8080,
            context_size: 4096,
            threads: 0,
            gpu_layers: 0,
            auto_start: false,
        }
    }
}

impl LocalAiSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.binary_path.trim().is_empty() {
            return Err("llama-server 路径不能为空".to_string());
        }
        let model_path = std::path::Path::new(self.model_path.trim());
        if self.model_path.trim().is_empty() {
            return Err("GGUF 模型路径不能为空".to_string());
        }
        if !model_path.is_file() {
            return Err(format!("GGUF 模型文件不存在：{}", model_path.display()));
        }
        if !model_path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("gguf"))
        {
            return Err("本地模型必须是 .gguf 文件".to_string());
        }
        if self.context_size < 512 {
            return Err("上下文长度不能小于 512".to_string());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalAiSession {
    pub pid: u32,
    pub port: u16,
    pub model_path: String,
}

impl LocalAiSession {
    pub fn endpoint(&self) -> String {
        format!("http://127.0.0.1:{}/v1/chat/completions", self.port)
    }
}

pub const LOCAL_AI_SUPPORTED: bool = cfg!(any(
    target_os = "macos",
    target_os = "windows",
    target_os = "linux"
));

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
mod desktop {
    use super::{LocalAiSession, LocalAiSettings, LOCAL_MODEL_ALIAS};
    use std::collections::VecDeque;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    const START_TIMEOUT: Duration = Duration::from_secs(180);

    #[derive(Default)]
    pub struct LocalAiManager {
        child: Option<Child>,
        session: Option<LocalAiSession>,
    }

    impl LocalAiManager {
        pub fn start(&mut self, settings: &LocalAiSettings) -> Result<LocalAiSession, String> {
            let cancel = AtomicBool::new(false);
            self.start_cancellable(settings, &cancel)
        }

        pub fn start_cancellable(
            &mut self,
            settings: &LocalAiSettings,
            cancel: &AtomicBool,
        ) -> Result<LocalAiSession, String> {
            self.start_with_timeout(settings, START_TIMEOUT, cancel)
        }

        fn start_with_timeout(
            &mut self,
            settings: &LocalAiSettings,
            timeout: Duration,
            cancel: &AtomicBool,
        ) -> Result<LocalAiSession, String> {
            settings.validate()?;
            self.stop()?;

            let port = available_port(settings.port)?;
            let binary = resolve_binary(settings.binary_path.trim())?;
            let threads = if settings.threads == 0 {
                std::thread::available_parallelism()
                    .map(|count| count.get().min(u16::MAX as usize) as u16)
                    .unwrap_or(4)
            } else {
                settings.threads
            };

            let mut command = Command::new(&binary);
            command.args([
                "--host",
                "127.0.0.1",
                "--port",
                &port.to_string(),
                "-m",
                settings.model_path.trim(),
                "-a",
                LOCAL_MODEL_ALIAS,
                "-c",
                &settings.context_size.to_string(),
                "--threads",
                &threads.to_string(),
            ]);
            if settings.gpu_layers > 0 {
                command.args(["-ngl", &settings.gpu_layers.to_string()]);
            }
            command.stdout(Stdio::piped()).stderr(Stdio::piped());
            hide_console_window(&mut command);

            let mut child = command.spawn().map_err(|error| {
                format!("无法启动 llama-server（{}）：{}", binary.display(), error)
            })?;
            let pid = child.id();
            let ready = Arc::new(AtomicBool::new(false));
            let log_tail = Arc::new(Mutex::new(VecDeque::with_capacity(40)));
            if let Some(stdout) = child.stdout.take() {
                drain_logs(stdout, "stdout", ready.clone(), log_tail.clone());
            }
            if let Some(stderr) = child.stderr.take() {
                drain_logs(stderr, "stderr", ready.clone(), log_tail.clone());
            }

            let started = Instant::now();
            loop {
                if cancel.load(Ordering::Relaxed) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("本地模型加载已取消".to_string());
                }
                if ready.load(Ordering::Relaxed) || health_ready(port) {
                    break;
                }
                if let Some(status) = child
                    .try_wait()
                    .map_err(|error| format!("检查 llama-server 状态失败：{error}"))?
                {
                    return Err(format!(
                        "llama-server 在模型就绪前退出（{}）{}",
                        status,
                        format_log_tail(&log_tail)
                    ));
                }
                if started.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "llama-server 加载模型超过 {} 秒{}",
                        timeout.as_secs(),
                        format_log_tail(&log_tail)
                    ));
                }
                std::thread::sleep(Duration::from_millis(100));
            }

            let session = LocalAiSession {
                pid,
                port,
                model_path: settings.model_path.trim().to_string(),
            };
            self.child = Some(child);
            self.session = Some(session.clone());
            Ok(session)
        }

        pub fn stop(&mut self) -> Result<(), String> {
            self.session = None;
            let Some(mut child) = self.child.take() else {
                return Ok(());
            };
            match child.try_wait() {
                Ok(Some(_)) => Ok(()),
                Ok(None) => {
                    child
                        .kill()
                        .map_err(|error| format!("停止 llama-server 失败：{error}"))?;
                    child
                        .wait()
                        .map_err(|error| format!("等待 llama-server 退出失败：{error}"))?;
                    Ok(())
                }
                Err(error) => Err(format!("检查 llama-server 状态失败：{error}")),
            }
        }

        /// 返回 Some 表示原本运行的服务已经退出。
        pub fn poll_exit(&mut self) -> Result<Option<String>, String> {
            let Some(child) = &mut self.child else {
                return Ok(None);
            };
            match child
                .try_wait()
                .map_err(|error| format!("检查 llama-server 状态失败：{error}"))?
            {
                Some(status) => {
                    self.child = None;
                    self.session = None;
                    Ok(Some(format!("llama-server 已退出：{status}")))
                }
                None => Ok(None),
            }
        }
    }

    impl Drop for LocalAiManager {
        fn drop(&mut self) {
            let _ = self.stop();
        }
    }

    fn available_port(requested: u16) -> Result<u16, String> {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), requested);
        let listener = TcpListener::bind(addr).map_err(|error| {
            if requested == 0 {
                format!("无法分配本地端口：{error}")
            } else {
                format!("本地端口 {requested} 已被占用：{error}")
            }
        })?;
        listener
            .local_addr()
            .map(|addr| addr.port())
            .map_err(|error| format!("读取本地端口失败：{error}"))
    }

    fn resolve_binary(input: &str) -> Result<PathBuf, String> {
        let path = Path::new(input);
        if (path.components().count() > 1 || path.is_absolute()) && !path.is_file() {
            return Err(format!("llama-server 文件不存在：{}", path.display()));
        }
        Ok(path.to_path_buf())
    }

    fn drain_logs<R: Read + Send + 'static>(
        stream: R,
        source: &'static str,
        ready: Arc<AtomicBool>,
        tail: Arc<Mutex<VecDeque<String>>>,
    ) {
        std::thread::spawn(move || {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                ::log::info!("[llama-server {source}] {trimmed}");
                let lower = trimmed.to_ascii_lowercase();
                if lower.contains("server is listening on")
                    || lower.contains("server listening on")
                    || lower.contains("starting the main loop")
                {
                    ready.store(true, Ordering::Relaxed);
                }
                let mut tail = tail.lock().unwrap();
                if tail.len() == 40 {
                    tail.pop_front();
                }
                tail.push_back(trimmed.to_string());
            }
        });
    }

    fn health_ready(port: u16) -> bool {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
        let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(80)) else {
            return false;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
        let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
        if stream
            .write_all(b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
            .is_err()
        {
            return false;
        }
        let mut response = [0_u8; 256];
        let Ok(read) = stream.read(&mut response) else {
            return false;
        };
        String::from_utf8_lossy(&response[..read]).contains(" 200 ")
    }

    fn format_log_tail(tail: &Arc<Mutex<VecDeque<String>>>) -> String {
        let tail = tail.lock().unwrap();
        if tail.is_empty() {
            String::new()
        } else {
            format!(
                "\n\n最近日志：\n{}",
                tail.iter().cloned().collect::<Vec<_>>().join("\n")
            )
        }
    }

    #[cfg(windows)]
    fn hide_console_window(command: &mut Command) {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }

    #[cfg(not(windows))]
    fn hide_console_window(_command: &mut Command) {}

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn allocates_an_ephemeral_local_port() {
            assert_ne!(available_port(0).unwrap(), 0);
        }

        #[cfg(unix)]
        #[test]
        fn starts_detects_ready_and_stops_a_server_process() {
            use std::os::unix::fs::PermissionsExt;

            let root = std::env::temp_dir()
                .join(format!("readwise-local-ai-manager-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            let binary = root.join("fake-llama-server");
            let model = root.join("model.gguf");
            std::fs::write(
                &binary,
                b"#!/bin/sh\necho 'server is listening on' >&2\nexec tail -f /dev/null\n",
            )
            .unwrap();
            let mut permissions = std::fs::metadata(&binary).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&binary, permissions).unwrap();
            std::fs::write(&model, b"fake model").unwrap();

            let settings = LocalAiSettings {
                binary_path: binary.display().to_string(),
                model_path: model.display().to_string(),
                port: 0,
                ..Default::default()
            };
            let mut manager = LocalAiManager::default();
            let cancel = AtomicBool::new(false);
            let session = manager
                .start_with_timeout(&settings, Duration::from_secs(3), &cancel)
                .unwrap();
            assert_ne!(session.pid, 0);
            assert_ne!(session.port, 0);
            assert_eq!(manager.poll_exit().unwrap(), None);
            manager.stop().unwrap();
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub use desktop::LocalAiManager;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_model_and_builds_endpoint() {
        let temp =
            std::env::temp_dir().join(format!("readwise-local-ai-{}.gguf", std::process::id()));
        std::fs::write(&temp, b"test").unwrap();
        let settings = LocalAiSettings {
            model_path: temp.display().to_string(),
            ..Default::default()
        };
        assert!(settings.validate().is_ok());
        assert_eq!(
            LocalAiSession {
                pid: 1,
                port: 9000,
                model_path: settings.model_path,
            }
            .endpoint(),
            "http://127.0.0.1:9000/v1/chat/completions"
        );
        std::fs::remove_file(temp).unwrap();
    }
}
