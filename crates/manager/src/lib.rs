//! TigerBeetle process manager with periodic backups via cron scheduling.
//!
//! This crate manages a TigerBeetle child process, streams its logs, and
//! periodically stops it to create compressed backups uploaded to S3 based
//! on a cron schedule.  The cron schedule can be changed at runtime via the
//! watch channel returned by [`ProcessManager::new`] — the scheduler is
//! rebuilt on the fly without restarting the TigerBeetle process.

pub mod backup;
pub mod error;
pub mod process;

use crate::error::Result;
use crate::process::TigerBeetleProcess;
use chrono::Utc;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast, watch};
use tokio_cron_scheduler::{Job, JobScheduler};
use tracing::{error, info};

pub use backup::{BackupConfig, BackupStrategy, S3BackupStrategy};
pub use error::ManagerError;

/// A log entry from the TigerBeetle process.
#[derive(Debug, Clone)]
pub struct LogEntry {
    /// ISO 8601 timestamp.
    pub timestamp: String,
    /// Log level.
    pub level: LogLevel,
    /// Log message.
    pub message: String,
}

/// Log level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    /// Informational log.
    Info,
    /// Warning log.
    Warn,
    /// Error log.
    Error,
}

/// Configuration for the TigerBeetle process manager.
#[derive(Debug, Clone)]
pub struct ManagerConfig {
    /// Path to the TigerBeetle executable.
    pub exe: String,
    /// Arguments to pass to the TigerBeetle process.
    pub args: Vec<String>,
    /// Path to the data file to backup.
    pub backup_file: PathBuf,
    /// S3 bucket name for backups.
    pub bucket: String,
}

/// Manager runtime state (exposed for API / gRPC access).
#[derive(Debug, Clone, Serialize)]
pub struct ManagerState {
    /// Whether the TigerBeetle process is currently running.
    pub process_running: bool,
    /// OS process ID (None if not running).
    pub pid: Option<u32>,
    /// TigerBeetle executable path.
    pub exe: String,
    /// Arguments passed to TigerBeetle.
    pub args: Vec<String>,
    /// TigerBeetle listen address.
    pub address: String,
    /// Whether backups are currently enabled.
    pub backups_enabled: bool,
    /// Cron schedule pattern (None = disabled).
    pub cron_schedule: Option<String>,
    /// Path to the backup file.
    pub backup_file: String,
    /// S3 bucket name.
    pub bucket: String,
    /// ISO 8601 timestamp of last successful backup.
    pub last_backup_at: Option<String>,
    /// Last backup error message.
    pub last_backup_error: Option<String>,
}

/// Main process manager — orchestrates TigerBeetle process + backups.
#[derive(Debug)]
pub struct ProcessManager<S: BackupStrategy> {
    config: ManagerConfig,
    backup_strategy: S,
    /// Shared state for gRPC access.
    pub manager_state: Arc<RwLock<ManagerState>>,
    /// Optional broadcast channel for streaming logs to clients.
    log_tx: Option<broadcast::Sender<LogEntry>>,
    /// Receives cron-schedule changes (None = disable backups).
    cron_rx: watch::Receiver<Option<String>>,
}

impl<S: BackupStrategy + 'static> ProcessManager<S> {
    /// Create a new process manager.
    ///
    /// `cron_rx` carries the live cron schedule.  The sender side should be
    /// held by the gRPC layer so it can push schedule changes at runtime.
    pub fn new(
        config: ManagerConfig,
        backup_strategy: S,
        log_tx: Option<broadcast::Sender<LogEntry>>,
        cron_rx: watch::Receiver<Option<String>>,
    ) -> Self {
        let address = config
            .args
            .iter()
            .find_map(|a| a.strip_prefix("--addresses="))
            .unwrap_or("")
            .to_string();

        let initial_cron = cron_rx.borrow().clone();

        let manager_state = Arc::new(RwLock::new(ManagerState {
            process_running: false,
            pid: None,
            exe: config.exe.clone(),
            args: config.args.clone(),
            address,
            backups_enabled: initial_cron.is_some(),
            cron_schedule: initial_cron,
            backup_file: config.backup_file.display().to_string(),
            bucket: config.bucket.clone(),
            last_backup_at: None,
            last_backup_error: None,
        }));

        ProcessManager {
            config,
            backup_strategy,
            manager_state,
            log_tx,
            cron_rx,
        }
    }

    /// Run the manager loop.
    ///
    /// Spawns the TigerBeetle process, starts a cron scheduler if a schedule
    /// is set, and watches for runtime schedule changes via the watch channel.
    /// Handles graceful shutdown on Ctrl-C.
    pub async fn run(self) -> Result<()> {
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);

        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            info!("Received Ctrl-C, shutting down...");
            shutdown_tx.send(true).expect("shutdown notify failed");
        });

        let backup_strategy = Arc::new(self.backup_strategy);
        let mut cron_rx = self.cron_rx;

        // Spawn TigerBeetle process.
        let mut child = TigerBeetleProcess::spawn(
            &self.config.exe,
            &self.config.args,
            shutdown_rx.clone(),
            self.log_tx.clone(),
        )
        .await?;

        {
            let mut ms = self.manager_state.write().await;
            ms.process_running = true;
            ms.pid = child.pid();
        }

        // Mark initial cron as "seen" so changed() only fires on future sends.
        let initial_cron: Option<String> = cron_rx.borrow_and_update().clone();

        let mut maybe_scheduler: Option<JobScheduler> = if let Some(ref pattern) = initial_cron {
            match Self::create_scheduler(
                pattern,
                backup_strategy.clone(),
                self.manager_state.clone(),
            )
            .await
            {
                Ok(s) => {
                    info!("Backup scheduler started with pattern '{}'", pattern);
                    Some(s)
                }
                Err(e) => {
                    error!("Failed to create initial backup scheduler: {}", e);
                    None
                }
            }
        } else {
            info!("Starting manager without backups");
            None
        };

        loop {
            tokio::select! {
                result = cron_rx.changed() => {
                    if result.is_err() {
                        // Sender dropped — shouldn't happen in normal operation.
                        break;
                    }

                    // Shut down old scheduler before rebuilding.
                    if let Some(mut s) = maybe_scheduler.take() {
                        s.shutdown().await.ok();
                    }

                    let new_schedule = cron_rx.borrow_and_update().clone();

                    if let Some(ref pattern) = new_schedule {
                        info!("Cron schedule updated to '{}', rebuilding scheduler", pattern);
                        match Self::create_scheduler(
                            pattern,
                            backup_strategy.clone(),
                            self.manager_state.clone(),
                        )
                        .await
                        {
                            Ok(s) => {
                                maybe_scheduler = Some(s);
                            }
                            Err(e) => {
                                error!("Failed to rebuild backup scheduler: {}", e);
                            }
                        }
                    } else {
                        info!("Backup cron schedule cleared — backups disabled");
                    }
                }

                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        info!("Shutdown requested: killing TigerBeetle and stopping scheduler");
                        if let Some(mut s) = maybe_scheduler.take() {
                            s.shutdown().await.ok();
                        }
                        child.kill_and_wait().await?;
                        break;
                    }
                }

                result = child.wait() => {
                    match result {
                        Ok(status) => {
                            if status.success() {
                                info!("TigerBeetle process exited successfully");
                            } else {
                                error!("TigerBeetle process exited with status: {}", status);
                            }
                        }
                        Err(e) => error!("Error waiting for TigerBeetle process: {}", e),
                    }
                    if let Some(mut s) = maybe_scheduler.take() {
                        s.shutdown().await.ok();
                    }
                    break;
                }
            }
        }

        info!("Manager process exiting");
        Ok(())
    }

    /// Build and start a cron scheduler for the given pattern.
    async fn create_scheduler(
        cron_pattern: &str,
        backup_strategy: Arc<S>,
        manager_state: Arc<RwLock<ManagerState>>,
    ) -> Result<JobScheduler> {
        let scheduler = JobScheduler::new()
            .await
            .map_err(|e| ManagerError::Backup(format!("failed to create scheduler: {e}")))?;

        let job = Job::new_async(cron_pattern, move |_uuid, mut _lock| {
            let backup_strategy = backup_strategy.clone();
            let manager_state = manager_state.clone();

            Box::pin(async move {
                let (enabled, backup_file, bucket) = {
                    let ms = manager_state.read().await;
                    (
                        ms.backups_enabled,
                        PathBuf::from(&ms.backup_file),
                        ms.bucket.clone(),
                    )
                };
                if !enabled {
                    info!("Backup job triggered but backups are disabled, skipping");
                    return;
                }

                info!(
                    "Cron backup job triggered - job {} {}",
                    _uuid,
                    &backup_file.display()
                );
                match backup_strategy.upload_backup(&bucket, &backup_file).await {
                    Ok(()) => {
                        let mut ms = manager_state.write().await;
                        ms.last_backup_at = Some(Utc::now().to_rfc3339());
                        ms.last_backup_error = None;
                        info!("Cron backup completed successfully");
                    }
                    Err(e) => {
                        let msg = format!("{:#}", e);
                        let mut ms = manager_state.write().await;
                        ms.last_backup_error = Some(msg.clone());
                        error!("Cron backup failed: {}", msg);
                    }
                }
                let next_tick = _lock.next_tick_for_job(_uuid).await;
                match next_tick {
                    Ok(Some(ts)) => info!("Next time for job is {:?}", ts),
                    _ => info!("Could not get next tick for 4s job"),
                }
            })
        })
        .map_err(|e| ManagerError::Backup(format!("invalid cron pattern: {e}")))?;

        scheduler
            .add(job)
            .await
            .map_err(|e| ManagerError::Backup(format!("failed to add job: {e}")))?;

        scheduler
            .start()
            .await
            .map_err(|e| ManagerError::Backup(format!("failed to start scheduler: {e}")))?;

        Ok(scheduler)
    }
}
