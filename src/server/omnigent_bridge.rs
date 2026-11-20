//! Supervises the omnigent bridge sidecar (`scripts/omnigent_bridge.py`) as a
//! child of the server: started with it, restarted with backoff when it exits,
//! and killed when the server drops it.

use std::io;
use std::process::{Child, Command};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tracing::{info, warn};

use crate::config::OmnigentBridgeConfig;

/// The bridge script shipped with this build, used when `script` is unset.
const EMBEDDED_SCRIPT: &str = include_str!("../../scripts/omnigent_bridge.py");
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const MIN_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// A run at least this long resets the backoff.
const STABLE_RUN: Duration = Duration::from_secs(60);

pub(crate) struct OmnigentBridge {
    /// Dropping the sender tells the supervisor thread to kill the child and exit.
    stop: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl OmnigentBridge {
    pub(crate) fn start(config: &OmnigentBridgeConfig) -> Option<Self> {
        if !config.enabled {
            return None;
        }
        let config = config.clone();
        Some(Self::supervise(
            move || bridge_command(&config).spawn(),
            MIN_BACKOFF,
        ))
    }

    fn supervise(
        mut spawn_child: impl FnMut() -> io::Result<Child> + Send + 'static,
        min_backoff: Duration,
    ) -> Self {
        let (stop, stopped) = mpsc::channel::<()>();
        let thread = std::thread::spawn(move || {
            let mut backoff = min_backoff;
            loop {
                let started = Instant::now();
                match spawn_child() {
                    Ok(mut child) => {
                        info!(pid = child.id(), "omnigent bridge started");
                        loop {
                            match stopped.recv_timeout(POLL_INTERVAL) {
                                Err(RecvTimeoutError::Timeout) => {}
                                _ => {
                                    let _ = child.kill();
                                    let _ = child.wait();
                                    return;
                                }
                            }
                            if let Ok(Some(status)) = child.try_wait() {
                                warn!(%status, "omnigent bridge exited; restarting");
                                break;
                            }
                        }
                    }
                    Err(err) => warn!(err = %err, "failed to start omnigent bridge"),
                }
                if started.elapsed() >= STABLE_RUN {
                    backoff = min_backoff;
                }
                match stopped.recv_timeout(backoff) {
                    Err(RecvTimeoutError::Timeout) => backoff = (backoff * 2).min(MAX_BACKOFF),
                    _ => return,
                }
            }
        });
        Self {
            stop: Some(stop),
            thread: Some(thread),
        }
    }
}

impl Drop for OmnigentBridge {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn bridge_command(config: &OmnigentBridgeConfig) -> Command {
    let mut command = Command::new("python3");
    if config.script.is_empty() {
        command.arg("-c").arg(EMBEDDED_SCRIPT);
    } else {
        command.arg(&config.script);
    }
    if let Ok(exe) = std::env::current_exe() {
        command.arg("--herdr").arg(exe);
    }
    if !config.default_space.is_empty() {
        command.arg("--default-space").arg(&config.default_space);
    }
    command.env(crate::api::SOCKET_PATH_ENV_VAR, crate::api::socket_path());
    command
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn disabled_config_starts_nothing() {
        assert!(OmnigentBridge::start(&OmnigentBridgeConfig::default()).is_none());
    }

    #[test]
    fn command_uses_script_path_or_embedded_source() {
        let mut config = OmnigentBridgeConfig {
            enabled: true,
            default_space: "work".into(),
            ..Default::default()
        };
        let args: Vec<_> = bridge_command(&config)
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[0], "-c");
        assert!(args[1].contains("omnigent"));
        assert!(args.ends_with(&["--default-space".into(), "work".into()]));

        config.script = "/x/bridge.py".into();
        let args: Vec<_> = bridge_command(&config)
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[0], "/x/bridge.py");
    }

    #[test]
    fn restarts_exited_child_and_kills_running_child_on_drop() {
        let spawns = Arc::new(AtomicUsize::new(0));
        let counter = spawns.clone();
        let bridge = OmnigentBridge::supervise(
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
                Command::new("sh").args(["-c", "exit 1"]).spawn()
            },
            Duration::from_millis(10),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while spawns.load(Ordering::SeqCst) < 3 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        drop(bridge);
        assert!(spawns.load(Ordering::SeqCst) >= 3);

        let (pid_tx, pid_rx) = mpsc::channel();
        let bridge = OmnigentBridge::supervise(
            move || {
                let child = Command::new("sleep").arg("30").spawn()?;
                let _ = pid_tx.send(child.id());
                Ok(child)
            },
            Duration::from_millis(10),
        );
        let pid = pid_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(bridge);
        // kill(pid, 0) fails once the child is gone and reaped.
        assert_ne!(unsafe { libc::kill(pid as i32, 0) }, 0);
    }
}
