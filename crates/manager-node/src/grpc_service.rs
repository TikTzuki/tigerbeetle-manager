//! gRPC service implementation bridging proto types to the manager crate.

use crate::proto::{
    self, AccountRecord, BackupStatus, FormatDataFileRequest, FormatDataFileResponse,
    GetBackupConfigRequest, GetBackupConfigResponse, GetStatusRequest, GetStatusResponse, LogEntry,
    LogLevel, ModifyBackupConfigRequest, ModifyBackupConfigResponse, ProcessState, ProcessStatus,
    ReadAccountsRequest, ReadAccountsResponse, ReadTransfersRequest, ReadTransfersResponse,
    StartBackupRequest, StartBackupResponse, StopBackupRequest, StopBackupResponse,
    StreamLogsRequest, TransferRecord, TriggerBackupRequest, TriggerBackupResponse,
    manager_node_server::ManagerNode,
};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use tb_manager::{BackupConfig, BackupStrategy, ManagerState, S3BackupStrategy};
use tb_reader::DataFileReader;
use tokio::sync::{Mutex, RwLock, broadcast, watch};
use tokio_stream::Stream;
use tonic::{Request, Response, Status};
use tracing::{debug, info, warn};

/// Shared state for the gRPC service.
#[derive(Debug, Clone)]
pub struct NodeState {
    /// Node identifier (e.g., "node-0").
    pub node_id: String,
    /// Shared manager state.
    pub manager_state: Arc<RwLock<ManagerState>>,
    /// Log broadcast channel (new entries pushed here).
    pub log_tx: broadcast::Sender<proto::LogEntry>,
    /// When the node started (for uptime calculation).
    pub started_at: chrono::DateTime<chrono::Utc>,
    /// Path to the TOML backup config file (None = not configured).
    pub backup_config_file: Option<PathBuf>,
    /// Mutex ensuring only one backup runs at a time.
    pub backup_lock: Arc<Mutex<()>>,
    /// Live cron-schedule sender — drives the ProcessManager scheduler.
    pub cron_schedule_tx: Arc<watch::Sender<Option<String>>>,
}

/// gRPC service for a single manager node.
#[derive(Debug)]
pub struct ManagerNodeService {
    state: NodeState,
}

impl ManagerNodeService {
    /// Create a new gRPC service with the given shared state.
    pub fn new(state: NodeState) -> Self {
        ManagerNodeService { state }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Persist `cron_schedule` into the TOML config file.
/// Logs a warning on error instead of failing — in-memory state is already set.
fn persist_cron(config_path: &PathBuf, cron: Option<&str>) {
    let result = (|| -> tb_manager::error::Result<()> {
        let mut cfg = BackupConfig::load_from_file(config_path)?;
        cfg.cron_schedule = cron.map(str::to_owned);
        cfg.save_to_file(config_path)
    })();
    if let Err(e) = result {
        warn!(
            "Could not persist cron schedule to {:?}: {}",
            config_path, e
        );
    } else {
        info!("Cron schedule {:?} persisted to {:?}", cron, config_path);
    }
}

// ---------------------------------------------------------------------------
// gRPC implementation
// ---------------------------------------------------------------------------

#[tonic::async_trait]
impl ManagerNode for ManagerNodeService {
    async fn get_status(
        &self,
        _request: Request<GetStatusRequest>,
    ) -> Result<Response<GetStatusResponse>, Status> {
        let ms = self.state.manager_state.read().await;

        let uptime = chrono::Utc::now()
            .signed_duration_since(self.state.started_at)
            .num_seconds()
            .max(0) as u64;

        let response = GetStatusResponse {
            node_id: self.state.node_id.clone(),
            process: Some(ProcessStatus {
                state: if ms.process_running {
                    ProcessState::Running.into()
                } else {
                    ProcessState::Stopped.into()
                },
                pid: ms.pid.unwrap_or(0),
                exe: ms.exe.clone(),
                args: ms.args.clone(),
                data_file: ms.backup_file.clone(),
                address: ms.address.clone(),
            }),
            backup: Some(BackupStatus {
                enabled: ms.backups_enabled,
                cron_schedule: ms.cron_schedule.clone().unwrap_or_default(),
                bucket: ms.bucket.clone(),
                last_backup_at: ms.last_backup_at.clone().unwrap_or_default(),
                last_error: ms.last_backup_error.clone().unwrap_or_default(),
            }),
            uptime_seconds: uptime,
        };

        Ok(Response::new(response))
    }

    async fn start_backup(
        &self,
        request: Request<StartBackupRequest>,
    ) -> Result<Response<StartBackupResponse>, Status> {
        let req = request.into_inner();

        if req.cron_schedule.trim().is_empty() {
            return Ok(Response::new(StartBackupResponse {
                success: false,
                message: "cron_schedule must not be empty".into(),
            }));
        }

        // Basic validation: 5 or 6 fields.
        let fields: Vec<&str> = req.cron_schedule.split_whitespace().collect();
        if fields.len() < 5 || fields.len() > 6 {
            return Ok(Response::new(StartBackupResponse {
                success: false,
                message: format!(
                    "Invalid cron pattern: expected 5-6 fields, got {}",
                    fields.len()
                ),
            }));
        }

        // Update in-memory state.
        {
            let mut ms = self.state.manager_state.write().await;
            ms.backups_enabled = true;
            ms.cron_schedule = Some(req.cron_schedule.clone());
        }

        // Persist cron to TOML so it survives restarts.
        if let Some(ref path) = self.state.backup_config_file {
            persist_cron(path, Some(&req.cron_schedule));
        } else {
            warn!(
                "No --backup-config-file configured; cron schedule '{}' is in-memory only",
                req.cron_schedule
            );
        }

        // Push the new schedule to ProcessManager — it will rebuild the
        // cron scheduler live without restarting TigerBeetle.
        self.state
            .cron_schedule_tx
            .send(Some(req.cron_schedule.clone()))
            .ok();

        // Trigger an immediate backup so the user doesn't have to wait for
        // the first scheduled interval.
        self.spawn_immediate_backup();

        debug!("Backups enabled with schedule '{}'", req.cron_schedule);
        Ok(Response::new(StartBackupResponse {
            success: true,
            message: format!(
                "Backups enabled with schedule '{}'. An immediate backup has been triggered.",
                req.cron_schedule
            ),
        }))
    }

    async fn stop_backup(
        &self,
        _request: Request<StopBackupRequest>,
    ) -> Result<Response<StopBackupResponse>, Status> {
        // Update in-memory state.
        {
            let mut ms = self.state.manager_state.write().await;
            ms.backups_enabled = false;
            ms.cron_schedule = None;
        }

        // Clear cron from TOML.
        if let Some(ref path) = self.state.backup_config_file {
            persist_cron(path, None);
        }

        // Signal ProcessManager to shut down the scheduler.
        self.state.cron_schedule_tx.send(None).ok();

        Ok(Response::new(StopBackupResponse {
            success: true,
            message: "Backups disabled".into(),
        }))
    }

    async fn trigger_backup(
        &self,
        _request: Request<TriggerBackupRequest>,
    ) -> Result<Response<TriggerBackupResponse>, Status> {
        // Reject if another backup is already running.
        let Ok(_guard) = self.state.backup_lock.try_lock() else {
            return Ok(Response::new(TriggerBackupResponse {
                success: false,
                message: "A backup is already in progress".into(),
            }));
        };
        drop(_guard);

        let (backup_file, bucket) = {
            let ms = self.state.manager_state.read().await;
            (ms.backup_file.clone(), ms.bucket.clone())
        };

        let response_msg = format!("Backup started for {:?} → s3://{}/", backup_file, bucket);

        let manager_state = self.state.manager_state.clone();
        let backup_config_file = self.state.backup_config_file.clone();
        let backup_lock = self.state.backup_lock.clone();

        tokio::spawn(async move {
            let _lock = backup_lock.lock().await;
            let strategy = S3BackupStrategy::new(backup_config_file).await;
            let path = PathBuf::from(&backup_file);

            match strategy.upload_backup(&bucket, &path).await {
                Ok(()) => {
                    let mut ms = manager_state.write().await;
                    ms.last_backup_at = Some(chrono::Utc::now().to_rfc3339());
                    ms.last_backup_error = None;
                    tracing::info!("One-off backup completed successfully");
                }
                Err(e) => {
                    let msg = format!("{:#}", e);
                    let mut ms = manager_state.write().await;
                    ms.last_backup_error = Some(msg.clone());
                    tracing::error!("One-off backup failed: {}", msg);
                }
            }
        });

        Ok(Response::new(TriggerBackupResponse {
            success: true,
            message: response_msg,
        }))
    }

    async fn modify_backup_config(
        &self,
        request: Request<ModifyBackupConfigRequest>,
    ) -> Result<Response<ModifyBackupConfigResponse>, Status> {
        let config_path = match &self.state.backup_config_file {
            Some(p) => p.clone(),
            None => {
                return Ok(Response::new(ModifyBackupConfigResponse {
                    success: false,
                    message: "No backup config file path configured. \
                              Start the node with --backup-config-file."
                        .into(),
                }));
            }
        };

        let req = request.into_inner();

        let mut cfg = BackupConfig::load_from_file(&config_path)
            .map_err(|e| Status::internal(format!("Failed to load existing backup config: {e}")))?;

        // Only override fields that are non-empty in the request.
        if !req.aws_endpoint_url.is_empty() {
            cfg.aws_endpoint_url = Some(req.aws_endpoint_url);
        }
        if !req.aws_access_key_id.is_empty() {
            cfg.aws_access_key_id = Some(req.aws_access_key_id);
        }
        if !req.aws_secret_access_key.is_empty() {
            cfg.aws_secret_access_key = Some(req.aws_secret_access_key);
        }
        if !req.aws_default_region.is_empty() {
            cfg.aws_default_region = Some(req.aws_default_region);
        }
        if !req.aws_request_checksum_calculation.is_empty() {
            cfg.aws_request_checksum_calculation = Some(req.aws_request_checksum_calculation);
        }
        if !req.aws_response_checksum_validation.is_empty() {
            cfg.aws_response_checksum_validation = Some(req.aws_response_checksum_validation);
        }
        if !req.bucket.is_empty() {
            cfg.bucket = Some(req.bucket.clone());
        }
        if !req.backup_file.is_empty() {
            cfg.backup_file = Some(req.backup_file.clone());
        }

        cfg.save_to_file(&config_path)
            .map_err(|e| Status::internal(format!("Failed to save backup config: {e}")))?;

        // Update live manager state for fields that take effect immediately.
        {
            let mut ms = self.state.manager_state.write().await;
            if let Some(ref b) = cfg.bucket {
                ms.bucket = b.clone();
            }
            if let Some(ref f) = cfg.backup_file {
                ms.backup_file = f.clone();
            }
        }

        debug!("Backup config updated at {:?}", config_path);
        Ok(Response::new(ModifyBackupConfigResponse {
            success: true,
            message: format!("Backup config saved to {:?}", config_path),
        }))
    }

    async fn get_backup_config(
        &self,
        _request: Request<GetBackupConfigRequest>,
    ) -> Result<Response<GetBackupConfigResponse>, Status> {
        let Some(ref config_path) = self.state.backup_config_file else {
            return Ok(Response::new(GetBackupConfigResponse {
                config_file_configured: false,
                aws_endpoint_url: String::new(),
                aws_access_key_id: String::new(),
                aws_secret_access_key: String::new(),
                aws_default_region: String::new(),
                aws_request_checksum_calculation: String::new(),
                aws_response_checksum_validation: String::new(),
                bucket: String::new(),
                backup_file: String::new(),
            }));
        };

        let cfg = BackupConfig::load_from_file(config_path)
            .map_err(|e| Status::internal(format!("Failed to load backup config: {e}")))?;

        Ok(Response::new(GetBackupConfigResponse {
            config_file_configured: true,
            aws_endpoint_url: cfg.aws_endpoint_url.unwrap_or_default(),
            aws_access_key_id: cfg.aws_access_key_id.unwrap_or_default(),
            aws_secret_access_key: cfg.aws_secret_access_key.unwrap_or_default(),
            aws_default_region: cfg.aws_default_region.unwrap_or_default(),
            aws_request_checksum_calculation: cfg
                .aws_request_checksum_calculation
                .unwrap_or_default(),
            aws_response_checksum_validation: cfg
                .aws_response_checksum_validation
                .unwrap_or_default(),
            bucket: cfg.bucket.unwrap_or_default(),
            backup_file: cfg.backup_file.unwrap_or_default(),
        }))
    }

    type StreamLogsStream = Pin<Box<dyn Stream<Item = Result<LogEntry, Status>> + Send + 'static>>;

    async fn stream_logs(
        &self,
        _request: Request<StreamLogsRequest>,
    ) -> Result<Response<Self::StreamLogsStream>, Status> {
        let mut rx = self.state.log_tx.subscribe();

        let stream = async_stream::stream! {
            loop {
                match rx.recv().await {
                    Ok(entry) => yield Ok(entry),
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        yield Ok(LogEntry {
                            timestamp: chrono::Utc::now().to_rfc3339(),
                            level: LogLevel::Warn.into(),
                            message: format!("... skipped {} log entries (slow consumer)", n),
                        });
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        };

        Ok(Response::new(Box::pin(stream)))
    }

    async fn read_accounts(
        &self,
        request: Request<ReadAccountsRequest>,
    ) -> Result<Response<ReadAccountsResponse>, Status> {
        let req = request.into_inner();
        let page = req.page as usize;
        let limit = (req.limit as usize).min(500).max(1);

        let data_file = self.state.manager_state.read().await.backup_file.clone();
        info!("ReadAccounts (combined): page={page} limit={limit} file=\"{data_file}\"");

        let accounts = tokio::task::spawn_blocking(move || -> Result<_, String> {
            let mut reader = DataFileReader::open(&data_file)
                .map_err(|e| format!("open data file \"{data_file}\": {e}"))?;
            reader
                .read_lsm_accounts(page, limit)
                .map_err(|e| format!("read accounts: {e}"))
        })
        .await
        .map_err(|e| Status::internal(format!("task join: {e}")))?
        .map_err(|e| {
            warn!("ReadAccounts error: {e}");
            Status::internal(e)
        })?;

        let records = accounts.into_iter().map(account_to_proto).collect();
        Ok(Response::new(ReadAccountsResponse {
            accounts: records,
            page: req.page,
            limit: req.limit,
        }))
    }

    async fn read_transfers(
        &self,
        request: Request<ReadTransfersRequest>,
    ) -> Result<Response<ReadTransfersResponse>, Status> {
        let req = request.into_inner();
        let page = req.page as usize;
        let limit = (req.limit as usize).min(500).max(1);

        let data_file = self.state.manager_state.read().await.backup_file.clone();
        info!("ReadTransfers (combined): page={page} limit={limit} file=\"{data_file}\"");

        let transfers = tokio::task::spawn_blocking(move || -> Result<_, String> {
            let mut reader = DataFileReader::open(&data_file)
                .map_err(|e| format!("open data file \"{data_file}\": {e}"))?;
            reader
                .read_lsm_transfers(page, limit)
                .map_err(|e| format!("read transfers: {e}"))
        })
        .await
        .map_err(|e| Status::internal(format!("task join: {e}")))?
        .map_err(|e| {
            warn!("ReadTransfers error: {e}");
            Status::internal(e)
        })?;

        let records = transfers.into_iter().map(transfer_to_proto).collect();
        Ok(Response::new(ReadTransfersResponse {
            transfers: records,
            page: req.page,
            limit: req.limit,
        }))
    }

    async fn read_lsm_accounts(
        &self,
        request: Request<ReadAccountsRequest>,
    ) -> Result<Response<ReadAccountsResponse>, Status> {
        let req = request.into_inner();
        let page = req.page as usize;
        let limit = (req.limit as usize).min(500).max(1);

        let data_file = self.state.manager_state.read().await.backup_file.clone();
        info!("ReadLsmAccounts: page={page} limit={limit} file=\"{data_file}\"");

        let accounts = tokio::task::spawn_blocking(move || -> Result<_, String> {
            let mut reader =
                DataFileReader::open(&data_file).map_err(|e| format!("open data file: {e}"))?;
            reader
                .read_lsm_accounts(page, limit)
                .map_err(|e| format!("read LSM accounts: {e}"))
        })
        .await
        .map_err(|e| Status::internal(format!("task join: {e}")))?
        .map_err(|e| {
            warn!("ReadLsmAccounts error: {e}");
            Status::internal(e)
        })?;

        let records = accounts.into_iter().map(account_to_proto).collect();
        Ok(Response::new(ReadAccountsResponse {
            accounts: records,
            page: req.page,
            limit: req.limit,
        }))
    }

    async fn read_lsm_transfers(
        &self,
        request: Request<ReadTransfersRequest>,
    ) -> Result<Response<ReadTransfersResponse>, Status> {
        let req = request.into_inner();
        let page = req.page as usize;
        let limit = (req.limit as usize).min(500).max(1);

        let data_file = self.state.manager_state.read().await.backup_file.clone();
        info!("ReadLsmTransfers: page={page} limit={limit} file=\"{data_file}\"");

        let transfers = tokio::task::spawn_blocking(move || -> Result<_, String> {
            let mut reader =
                DataFileReader::open(&data_file).map_err(|e| format!("open data file: {e}"))?;
            reader
                .read_lsm_transfers(page, limit)
                .map_err(|e| format!("read LSM transfers: {e}"))
        })
        .await
        .map_err(|e| Status::internal(format!("task join: {e}")))?
        .map_err(|e| {
            warn!("ReadLsmTransfers error: {e}");
            Status::internal(e)
        })?;

        let records = transfers.into_iter().map(transfer_to_proto).collect();
        Ok(Response::new(ReadTransfersResponse {
            transfers: records,
            page: req.page,
            limit: req.limit,
        }))
    }

    async fn read_wal_accounts(
        &self,
        request: Request<ReadAccountsRequest>,
    ) -> Result<Response<ReadAccountsResponse>, Status> {
        let req = request.into_inner();
        let page = req.page as usize;
        let limit = (req.limit as usize).min(500).max(1);

        let data_file = self.state.manager_state.read().await.backup_file.clone();
        info!("ReadWalAccounts: page={page} limit={limit} file=\"{data_file}\"");

        let accounts = tokio::task::spawn_blocking(move || -> Result<_, String> {
            let mut reader =
                DataFileReader::open(&data_file).map_err(|e| format!("open data file: {e}"))?;
            reader
                .read_wal_accounts(page, limit)
                .map_err(|e| format!("read WAL accounts: {e}"))
        })
        .await
        .map_err(|e| Status::internal(format!("task join: {e}")))?
        .map_err(|e| {
            warn!("ReadWalAccounts error: {e}");
            Status::internal(e)
        })?;

        let records = accounts.into_iter().map(account_to_proto).collect();
        Ok(Response::new(ReadAccountsResponse {
            accounts: records,
            page: req.page,
            limit: req.limit,
        }))
    }

    async fn read_wal_transfers(
        &self,
        request: Request<ReadTransfersRequest>,
    ) -> Result<Response<ReadTransfersResponse>, Status> {
        let req = request.into_inner();
        let page = req.page as usize;
        let limit = (req.limit as usize).min(500).max(1);

        let data_file = self.state.manager_state.read().await.backup_file.clone();
        info!("ReadWalTransfers: page={page} limit={limit} file=\"{data_file}\"");

        let transfers = tokio::task::spawn_blocking(move || -> Result<_, String> {
            let mut reader =
                DataFileReader::open(&data_file).map_err(|e| format!("open data file: {e}"))?;
            reader
                .read_wal_transfers(page, limit)
                .map_err(|e| format!("read WAL transfers: {e}"))
        })
        .await
        .map_err(|e| Status::internal(format!("task join: {e}")))?
        .map_err(|e| {
            warn!("ReadWalTransfers error: {e}");
            Status::internal(e)
        })?;

        let records = transfers.into_iter().map(transfer_to_proto).collect();
        Ok(Response::new(ReadTransfersResponse {
            transfers: records,
            page: req.page,
            limit: req.limit,
        }))
    }

    async fn format_data_file(
        &self,
        request: Request<FormatDataFileRequest>,
    ) -> Result<Response<FormatDataFileResponse>, Status> {
        let req = request.into_inner();

        // Safety check: refuse if TigerBeetle is still running.
        {
            let ms = self.state.manager_state.read().await;
            if ms.process_running {
                return Ok(Response::new(FormatDataFileResponse {
                    success: false,
                    message: "TigerBeetle process is still running. \
                              Stop the node before formatting a data file."
                        .into(),
                    data_file_path: String::new(),
                }));
            }
        }

        // Resolve target data file path: request field takes priority, then configured path.
        let data_file_path = if !req.data_file_path.is_empty() {
            req.data_file_path.clone()
        } else {
            self.state.manager_state.read().await.backup_file.clone()
        };

        if data_file_path.is_empty() {
            return Ok(Response::new(FormatDataFileResponse {
                success: false,
                message: "No data file path provided and none is configured on this node.".into(),
                data_file_path: String::new(),
            }));
        }

        // Retrieve the TigerBeetle executable path from manager state.
        let exe = self.state.manager_state.read().await.exe.clone();

        // Build `tigerbeetle format` arguments.
        let mut cmd_args: Vec<String> = vec![
            "format".into(),
            format!("--cluster={}", req.cluster_id),
            format!("--replica={}", req.replica),
            format!("--replica-count={}", req.replica_count),
        ];
        if !req.size.is_empty() {
            cmd_args.push(format!("--size={}", req.size));
        }
        cmd_args.push(data_file_path.clone());

        info!("FormatDataFile: {} {}", exe, cmd_args.join(" "));

        let output = tokio::process::Command::new(&exe)
            .args(&cmd_args)
            .output()
            .await
            .map_err(|e| Status::internal(format!("failed to spawn tigerbeetle: {e}")))?;

        if output.status.success() {
            info!("FormatDataFile succeeded: {}", data_file_path);
            Ok(Response::new(FormatDataFileResponse {
                success: true,
                message: format!(
                    "Data file formatted successfully: cluster={} replica={}/{} path={}",
                    req.cluster_id, req.replica, req.replica_count, data_file_path
                ),
                data_file_path,
            }))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let combined = format!("{}{}", stdout, stderr).trim().to_string();
            warn!(
                "FormatDataFile failed (exit {:?}): {}",
                output.status.code(),
                combined
            );
            Ok(Response::new(FormatDataFileResponse {
                success: false,
                message: format!(
                    "tigerbeetle format failed (exit {:?}): {}",
                    output.status.code(),
                    combined
                ),
                data_file_path: String::new(),
            }))
        }
    }
}

// ---------------------------------------------------------------------------
// Private helpers on ManagerNodeService
// ---------------------------------------------------------------------------

fn account_to_proto(a: tb_reader::Account) -> AccountRecord {
    AccountRecord {
        id: a.id.to_string(),
        debits_pending: a.debits_pending.to_string(),
        debits_posted: a.debits_posted.to_string(),
        credits_pending: a.credits_pending.to_string(),
        credits_posted: a.credits_posted.to_string(),
        user_data_128: a.user_data_128.to_string(),
        user_data_64: a.user_data_64,
        user_data_32: a.user_data_32,
        ledger: a.ledger,
        code: a.code as u32,
        flags: a.flags.raw() as u32,
        timestamp: a.timestamp,
    }
}

fn transfer_to_proto(t: tb_reader::Transfer) -> TransferRecord {
    TransferRecord {
        id: t.id.to_string(),
        debit_account_id: t.debit_account_id.to_string(),
        credit_account_id: t.credit_account_id.to_string(),
        amount: t.amount.to_string(),
        pending_id: t.pending_id.to_string(),
        user_data_128: t.user_data_128.to_string(),
        user_data_64: t.user_data_64,
        user_data_32: t.user_data_32,
        timeout: t.timeout,
        ledger: t.ledger,
        code: t.code as u32,
        flags: t.flags.raw() as u32,
        timestamp: t.timestamp,
    }
}

impl ManagerNodeService {
    /// Spawn a background backup task using the current node credentials.
    /// Uses try_lock so it silently skips if a backup is already in progress.
    fn spawn_immediate_backup(&self) {
        let Ok(_guard) = self.state.backup_lock.try_lock() else {
            info!("Immediate backup skipped — another backup is already in progress");
            return;
        };
        drop(_guard);

        let manager_state = self.state.manager_state.clone();
        let backup_config_file = self.state.backup_config_file.clone();
        let backup_lock = self.state.backup_lock.clone();

        tokio::spawn(async move {
            let _lock = backup_lock.lock().await;
            let (backup_file, bucket) = {
                let ms = manager_state.read().await;
                (ms.backup_file.clone(), ms.bucket.clone())
            };

            let strategy = S3BackupStrategy::new(backup_config_file).await;
            let path = PathBuf::from(&backup_file);

            match strategy.upload_backup(&bucket, &path).await {
                Ok(()) => {
                    let mut ms = manager_state.write().await;
                    ms.last_backup_at = Some(chrono::Utc::now().to_rfc3339());
                    ms.last_backup_error = None;
                    tracing::info!("Immediate backup (on schedule set) completed successfully");
                }
                Err(e) => {
                    let msg = format!("{:#}", e);
                    let mut ms = manager_state.write().await;
                    ms.last_backup_error = Some(msg.clone());
                    tracing::error!("Immediate backup (on schedule set) failed: {}", msg);
                }
            }
        });
    }
}
