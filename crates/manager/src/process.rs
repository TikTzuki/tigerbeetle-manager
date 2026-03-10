//! TigerBeetle child process management with log streaming.

use crate::error::{ManagerError, Result};
use crate::{LogEntry, LogLevel};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{broadcast, watch};
use tracing::{error, info};

/// Manages a TigerBeetle child process with stdout/stderr streaming.
#[derive(Debug)]
pub struct TigerBeetleProcess {
    child: Child,
}

impl TigerBeetleProcess {
    /// Spawn a TigerBeetle process with the given executable and arguments.
    ///
    /// Stdout and stderr are streamed to the tracing logger and optionally to a broadcast channel.
    pub async fn spawn(
        exe: &str,
        args: &[String],
        shutdown_rx: watch::Receiver<bool>,
        log_tx: Option<broadcast::Sender<LogEntry>>,
    ) -> Result<Self> {
        info!("Spawning TigerBeetle: {} {:?}", exe, args);
        let mut child = Command::new(exe)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| ManagerError::Process(format!("failed to spawn '{}': {}", exe, e)))?;

        // Attach stdout reader.
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ManagerError::Process("child stdout missing".into()))?;
        let mut stdout_lines = BufReader::new(stdout).lines();
        let shutdown_clone = shutdown_rx.clone();
        let log_tx_clone = log_tx.clone();
        tokio::spawn(async move {
            loop {
                if *shutdown_clone.borrow() {
                    break;
                }
                match stdout_lines.next_line().await {
                    Ok(Some(line)) => {
                        info!(target: "tigerbeetle", "{}", line);
                        if let Some(ref tx) = log_tx_clone {
                            let _ = tx.send(LogEntry {
                                timestamp: chrono::Utc::now().to_rfc3339(),
                                level: LogLevel::Info,
                                message: line,
                            });
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        error!("error reading stdout: {}", e);
                        break;
                    }
                }
            }
        });

        // Attach stderr reader.
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| ManagerError::Process("child stderr missing".into()))?;
        let mut stderr_lines = BufReader::new(stderr).lines();
        let shutdown_clone = shutdown_rx;
        tokio::spawn(async move {
            loop {
                if *shutdown_clone.borrow() {
                    break;
                }
                match stderr_lines.next_line().await {
                    Ok(Some(line)) => {
                        info!(target: "tigerbeetle", "{}", line); // Tigerbeetle send log info into error channel
                        if let Some(ref tx) = log_tx {
                            let _ = tx.send(LogEntry {
                                timestamp: chrono::Utc::now().to_rfc3339(),
                                level: LogLevel::Error,
                                message: line,
                            });
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        error!("error reading stderr: {}", e);
                        break;
                    }
                }
            }
        });

        Ok(TigerBeetleProcess { child })
    }

    /// Get the OS process ID, if available.
    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    /// Kill the child process and wait for it to exit.
    pub async fn kill_and_wait(mut self) -> Result<()> {
        if let Some(id) = self.child.id() {
            info!("Killing TigerBeetle process (pid {})", id);
        }
        match self.child.kill().await {
            Ok(()) => {
                let _ = self.child.wait().await;
            }
            Err(e) => {
                error!("kill() error (maybe already exited): {}", e);
                let _ = self.child.wait().await;
            }
        }
        Ok(())
    }

    /// Wait for the child process to exit.
    pub async fn wait(&mut self) -> Result<std::process::ExitStatus> {
        self.child
            .wait()
            .await
            .map_err(|e| ManagerError::Process(format!("wait failed: {e}")))
    }
}
