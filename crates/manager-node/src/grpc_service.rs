//! gRPC service implementation bridging proto types to the manager crate.

use crate::proto::{
    self, AccountRecord, BackupStatus, DataFileCapacity, ExecuteMigrationRequest,
    FormatDataFileRequest, FormatDataFileResponse, GetBackupConfigRequest, GetBackupConfigResponse,
    GetMigrationAccountsRequest, GetMigrationAccountsResponse,
    GetMigrationSyntheticTransfersRequest, GetMigrationSyntheticTransfersResponse,
    GetStatusRequest, GetStatusResponse, LedgerSummary, LogEntry, LogLevel, MigrationProgress,
    ModifyBackupConfigRequest, ModifyBackupConfigResponse, PlanMigrationRequest,
    PlanMigrationResponse, ProcessState, ProcessStatus, ReadAccountsRequest, ReadAccountsResponse,
    ReadTransfersRequest, ReadTransfersResponse, StartBackupRequest, StartBackupResponse,
    StopBackupRequest, StopBackupResponse, StreamLogsRequest, SyntheticTransferRecord,
    TransferRecord, TriggerBackupRequest, TriggerBackupResponse, manager_node_server::ManagerNode,
};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use tb_manager::{BackupConfig, BackupStrategy, ManagerState, S3BackupStrategy};
use tb_reader::DataFileReader;
use tokio::sync::{Mutex, RwLock, broadcast, watch};
use tokio_stream::Stream;
use tonic::{Request, Response, Status};
use tracing::{info, warn};

/// Cached migration plan for paginated drill-down queries.
#[derive(Debug, Clone)]
pub struct CachedMigrationPlan {
    /// All accounts from the data file.
    pub accounts: Vec<tb_reader::Account>,
    /// Synthetic transfers computed by BalancePlan.
    pub synthetic_transfers: Vec<tb_compressor::SyntheticTransfer>,
    /// Windowed transfers (empty for snapshot-only migrations).
    pub windowed_transfers: Vec<tb_reader::Transfer>,
}

/// Shared state for the gRPC service.
#[derive(Debug, Clone)]
pub struct NodeState {
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
    /// Cached migration plan populated by PlanMigration for drill-down RPCs.
    pub cached_migration: Arc<RwLock<Option<CachedMigrationPlan>>>,
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

        let data_file = ms.backup_file.clone();

        // Read data file capacity stats + replica info (best-effort, non-fatal).
        let (capacity, replica_info) = {
            let df = data_file.clone();
            tokio::task::spawn_blocking(move || {
                DataFileReader::open(&df)
                    .map(|mut r| {
                        let capacity = r.capacity_stats().ok();
                        let replica_info = r.read_replica_info().ok();
                        (capacity, replica_info)
                    })
                    .unwrap_or((None, None))
            })
            .await
            .unwrap_or((None, None))
        };

        let capacity = capacity.map(|stats| DataFileCapacity {
            data_file_size_bytes: stats.data_file_size_bytes,
            grid_blocks_total: stats.grid_blocks_total,
            grid_blocks_used: stats.grid_blocks_used,
        });

        let node_id = replica_info
            .as_ref()
            .and_then(|i| i.replica)
            .map_or_else(|| "unknown".to_string(), |r| r.to_string());

        let response = GetStatusResponse {
            node_id,
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
            capacity,
            cluster_id: replica_info.map_or_else(String::new, |i| i.cluster_id.to_string()),
            replica: replica_info
                .and_then(|i| i.replica)
                .map_or(-1i32, |r| r as i32),
            replica_count: replica_info.map_or(0u32, |i| i.replica_count as u32),
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

        info!("Backups enabled with schedule '{}'", req.cron_schedule);
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

        info!("Backup config updated at {:?}", config_path);
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

    async fn plan_migration(
        &self,
        request: Request<PlanMigrationRequest>,
    ) -> Result<Response<PlanMigrationResponse>, Status> {
        let req = request.into_inner();
        let cutoff_ts = req.cutoff_ts;
        let data_file = self.state.manager_state.read().await.backup_file.clone();
        info!("PlanMigration: reading accounts from {data_file} cutoff_ts={cutoff_ts}");

        let result = tokio::task::spawn_blocking(
            move || -> Result<(PlanMigrationResponse, CachedMigrationPlan), String> {
                // Read all accounts (LSM + WAL merged).
                let accounts = read_all_accounts(&data_file)?;

                let total_accounts = accounts.len() as u64;

                // Count accounts with non-zero pending balances.
                let pending = accounts
                    .iter()
                    .filter(|a| a.debits_pending > 0 || a.credits_pending > 0)
                    .count() as u64;

                // Collect pending accounts for the response.
                let pending_accounts: Vec<AccountRecord> = accounts
                    .iter()
                    .filter(|a| a.debits_pending > 0 || a.credits_pending > 0)
                    .cloned()
                    .map(account_to_proto)
                    .collect();

                // Compute per-ledger summaries.
                let mut ledger_map: std::collections::BTreeMap<u32, (u64, u128, u128)> =
                    std::collections::BTreeMap::new();
                for a in &accounts {
                    let entry = ledger_map.entry(a.ledger).or_insert((0, 0, 0));
                    entry.0 += 1;
                    entry.1 += a.debits_posted;
                    entry.2 += a.credits_posted;
                }
                let ledger_count = ledger_map.len() as u32;
                let ledger_summaries: Vec<LedgerSummary> = ledger_map
                    .into_iter()
                    .map(|(ledger, (count, debits, credits))| LedgerSummary {
                        ledger,
                        account_count: count,
                        total_debits_posted: debits.to_string(),
                        total_credits_posted: credits.to_string(),
                    })
                    .collect();

                // Clone accounts before BalancePlan::build*() consumes them.
                let accounts_for_cache = accounts.clone();

                // Build plan (windowed or snapshot) to get transfer counts.
                let (plan, windowed_count) = if cutoff_ts > 0 {
                    let windowed = read_all_transfers_since(&data_file, cutoff_ts)?;
                    let wcount = windowed.len() as u64;
                    let p = tb_compressor::BalancePlan::build_windowed(
                        accounts,
                        windowed.clone(),
                        cutoff_ts,
                    );
                    (p, wcount)
                } else {
                    let p = tb_compressor::BalancePlan::build(accounts);
                    (p, 0u64)
                };

                let synthetic_transfers_count = plan.total_transfers() as u64;

                let windowed_for_cache = plan.windowed_transfers.clone();
                let cached = CachedMigrationPlan {
                    accounts: accounts_for_cache,
                    synthetic_transfers: plan.synthetic_transfers.clone(),
                    windowed_transfers: windowed_for_cache,
                };

                let response = PlanMigrationResponse {
                    accounts: total_accounts,
                    pending_transfers: pending,
                    synthetic_transfers: synthetic_transfers_count,
                    safe: pending == 0,
                    ledgers: ledger_count,
                    ledger_summaries,
                    pending_accounts,
                    windowed_transfers: windowed_count,
                };

                Ok((response, cached))
            },
        )
        .await
        .map_err(|e| Status::internal(format!("task join: {e}")))?
        .map_err(|e| {
            warn!("PlanMigration error: {e}");
            Status::internal(e)
        })?;

        // Store the cached plan for drill-down RPCs.
        *self.state.cached_migration.write().await = Some(result.1);

        Ok(Response::new(result.0))
    }

    async fn get_migration_accounts(
        &self,
        request: Request<GetMigrationAccountsRequest>,
    ) -> Result<Response<GetMigrationAccountsResponse>, Status> {
        let req = request.into_inner();
        let page = req.page as usize;
        let limit = (req.limit as usize).min(500).max(1);

        let cache_guard = self.state.cached_migration.read().await;
        let cached = cache_guard.as_ref().ok_or_else(|| {
            Status::not_found("No cached migration plan. Run PlanMigration first.")
        })?;

        let filter = req.filter;

        // Apply filters.
        let matched: Vec<&tb_reader::Account> = cached
            .accounts
            .iter()
            .filter(|a| {
                if let Some(ref f) = filter {
                    if let Some(ref id_str) = f.id {
                        if let Ok(id_val) = id_str.parse::<u128>() {
                            if a.id != id_val {
                                return false;
                            }
                        }
                    }
                    if let Some(ledger) = f.ledger {
                        if a.ledger != ledger {
                            return false;
                        }
                    }
                    if let Some(code) = f.code {
                        if a.code as u32 != code {
                            return false;
                        }
                    }
                    if let Some(flags) = f.flags {
                        if a.flags.raw() as u32 != flags {
                            return false;
                        }
                    }
                    if let Some(ud32) = f.user_data_32 {
                        if a.user_data_32 != ud32 {
                            return false;
                        }
                    }
                    if let Some(ud64) = f.user_data_64 {
                        if a.user_data_64 != ud64 {
                            return false;
                        }
                    }
                    if let Some(ref ud128_str) = f.user_data_128 {
                        if let Ok(ud128_val) = ud128_str.parse::<u128>() {
                            if a.user_data_128 != ud128_val {
                                return false;
                            }
                        }
                    }
                }
                true
            })
            .collect();

        let total_count = matched.len() as u64;
        let records: Vec<AccountRecord> = matched
            .into_iter()
            .skip(page * limit)
            .take(limit)
            .cloned()
            .map(account_to_proto)
            .collect();

        Ok(Response::new(GetMigrationAccountsResponse {
            accounts: records,
            page: req.page,
            limit: req.limit,
            total_count,
        }))
    }

    async fn get_migration_synthetic_transfers(
        &self,
        request: Request<GetMigrationSyntheticTransfersRequest>,
    ) -> Result<Response<GetMigrationSyntheticTransfersResponse>, Status> {
        let req = request.into_inner();
        let page = req.page as usize;
        let limit = (req.limit as usize).min(500).max(1);

        let cache_guard = self.state.cached_migration.read().await;
        let cached = cache_guard.as_ref().ok_or_else(|| {
            Status::not_found("No cached migration plan. Run PlanMigration first.")
        })?;

        let matched: Vec<&tb_compressor::SyntheticTransfer> = cached
            .synthetic_transfers
            .iter()
            .filter(|t| {
                if let Some(ledger) = req.ledger {
                    t.ledger == ledger
                } else {
                    true
                }
            })
            .collect();

        let total_count = matched.len() as u64;
        let records: Vec<SyntheticTransferRecord> = matched
            .into_iter()
            .skip(page * limit)
            .take(limit)
            .map(|t| SyntheticTransferRecord {
                id: t.id.to_string(),
                debit_account_id: t.debit_account_id.to_string(),
                credit_account_id: t.credit_account_id.to_string(),
                amount: t.amount.to_string(),
                ledger: t.ledger,
                code: t.code as u32,
                timestamp: t.timestamp,
            })
            .collect();

        Ok(Response::new(GetMigrationSyntheticTransfersResponse {
            transfers: records,
            page: req.page,
            limit: req.limit,
            total_count,
        }))
    }

    type ExecuteMigrationStream =
        Pin<Box<dyn Stream<Item = Result<MigrationProgress, Status>> + Send + 'static>>;

    async fn execute_migration(
        &self,
        request: Request<ExecuteMigrationRequest>,
    ) -> Result<Response<Self::ExecuteMigrationStream>, Status> {
        let req = request.into_inner();

        if req.new_addresses.is_empty() {
            return Err(Status::invalid_argument("new_addresses must not be empty"));
        }

        let cutoff_ts = req.cutoff_ts;
        let data_file = self.state.manager_state.read().await.backup_file.clone();
        info!(
            "ExecuteMigration: cluster_id={} addresses={} source={} cutoff_ts={cutoff_ts}",
            req.new_cluster_id, req.new_addresses, data_file
        );

        // Read accounts and (optionally) windowed transfers — merge LSM + WAL (blocking).
        let plan = tokio::task::spawn_blocking({
            let df = data_file.clone();
            move || -> Result<tb_compressor::BalancePlan, String> {
                let accounts = read_all_accounts(&df)?;

                // Safety check: refuse if any account has pending balances.
                let pending_count = accounts
                    .iter()
                    .filter(|a| a.debits_pending > 0 || a.credits_pending > 0)
                    .count();
                if pending_count > 0 {
                    return Err(format!(
                        "{pending_count} account(s) have non-zero pending balances. \
                         Void all pending transfers before migration."
                    ));
                }

                if cutoff_ts > 0 {
                    let windowed = read_all_transfers_since(&df, cutoff_ts)?;
                    Ok(tb_compressor::BalancePlan::build_windowed(
                        accounts, windowed, cutoff_ts,
                    ))
                } else {
                    Ok(tb_compressor::BalancePlan::build(accounts))
                }
            }
        })
        .await
        .map_err(|e| Status::internal(format!("task join: {e}")))?
        .map_err(|e| Status::failed_precondition(e))?;

        info!(
            "ExecuteMigration plan: {} genesis + {} regular accounts, \
             {} synthetic transfers, {} windowed transfers",
            plan.genesis_accounts.len(),
            plan.regular_accounts.len(),
            plan.total_transfers(),
            plan.total_windowed_transfers(),
        );

        let new_cluster_id: u128 = req.new_cluster_id.parse().map_err(|_| {
            Status::invalid_argument(format!(
                "invalid new_cluster_id {:?}: expected decimal u128",
                req.new_cluster_id
            ))
        })?;
        let new_addresses = req.new_addresses.clone();

        // Create progress channel.
        let (progress_tx, mut progress_rx) =
            tokio::sync::mpsc::channel::<tb_compressor::ImportProgress>(32);

        // Spawn the import task.
        tokio::spawn(async move {
            let importer =
                match tb_compressor::Importer::connect(new_cluster_id, &new_addresses).await {
                    Ok(imp) => imp,
                    Err(e) => {
                        tracing::error!("ExecuteMigration: failed to connect to new cluster: {e}");
                        return;
                    }
                };

            if let Err(e) = importer.import_all_with_progress(&plan, progress_tx).await {
                tracing::error!("ExecuteMigration: import failed: {e}");
            }
        });

        // Stream progress back to the client.
        let stream = async_stream::stream! {
            while let Some(p) = progress_rx.recv().await {
                yield Ok(MigrationProgress {
                    phase: p.phase,
                    imported: p.imported,
                    total: p.total,
                    done: false,
                    error: String::new(),
                });
            }
            // Channel closed — import is done (or errored).
            yield Ok(MigrationProgress {
                phase: "done".into(),
                imported: 0,
                total: 0,
                done: true,
                error: String::new(),
            });
        };

        Ok(Response::new(Box::pin(stream)))
    }
}

// ---------------------------------------------------------------------------
// Private helpers on ManagerNodeService
// ---------------------------------------------------------------------------

/// Read ALL accounts from a data file by merging LSM (checkpointed) and WAL
/// (pre-checkpoint) sources. WAL accounts override LSM accounts with the same
/// ID since the WAL has more recent balances.
fn read_all_accounts(data_file: &str) -> Result<Vec<tb_reader::Account>, String> {
    use std::collections::HashMap;

    let mut reader = DataFileReader::open(data_file).map_err(|e| format!("open data file: {e}"))?;

    // 1. Read LSM accounts (checkpointed).
    let lsm_accounts: Vec<tb_reader::Account> = match reader.iter_accounts() {
        Ok(iter) => iter
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("read LSM accounts: {e}"))?,
        Err(tb_reader::ReaderError::NotCheckpointed { .. }) => vec![],
        Err(e) => return Err(format!("iter LSM accounts: {e}")),
    };

    // 2. Read WAL accounts (post-checkpoint, not yet flushed to LSM).
    let wal_accounts: Vec<tb_reader::Account> = match reader.iter_wal_accounts() {
        Ok(iter) => iter
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("read WAL accounts: {e}"))?,
        Err(e) => return Err(format!("iter WAL accounts: {e}")),
    };

    if wal_accounts.is_empty() {
        return Ok(lsm_accounts);
    }

    // 3. Merge: start with LSM, then override/add from WAL (WAL has latest state).
    let mut by_id: HashMap<u128, tb_reader::Account> =
        lsm_accounts.into_iter().map(|a| (a.id, a)).collect();
    for acc in wal_accounts {
        by_id.insert(acc.id, acc); // WAL overrides LSM for same ID
    }

    Ok(by_id.into_values().collect())
}

/// Read all transfers with `timestamp >= cutoff_ts` from a data file.
///
/// Merges LSM (checkpointed) and WAL (pre-checkpoint) sources. WAL overrides
/// LSM for the same transfer ID. Result is sorted by timestamp (ascending) —
/// required for the `imported` flag constraint.
fn read_all_transfers_since(
    data_file: &str,
    cutoff_ts: u64,
) -> Result<Vec<tb_reader::Transfer>, String> {
    use std::collections::HashMap;

    let mut reader = DataFileReader::open(data_file).map_err(|e| format!("open data file: {e}"))?;

    // 1. Read LSM transfers (checkpointed).
    let lsm_transfers: Vec<tb_reader::Transfer> = match reader.iter_transfers() {
        Ok(iter) => iter
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("read LSM transfers: {e}"))?,
        Err(tb_reader::ReaderError::NotCheckpointed { .. }) => vec![],
        Err(e) => return Err(format!("iter LSM transfers: {e}")),
    };

    // 2. Read WAL transfers (post-checkpoint, not yet flushed to LSM).
    let wal_transfers: Vec<tb_reader::Transfer> = match reader.iter_wal_transfers() {
        Ok(iter) => iter
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("read WAL transfers: {e}"))?,
        Err(e) => return Err(format!("iter WAL transfers: {e}")),
    };

    // 3. Merge: start with LSM, then override/add from WAL (WAL has latest state).
    let mut by_id: HashMap<u128, tb_reader::Transfer> =
        lsm_transfers.into_iter().map(|t| (t.id, t)).collect();
    for t in wal_transfers {
        by_id.insert(t.id, t); // WAL overrides LSM for same ID
    }

    // 4. Filter to time window and sort by timestamp (required for imported flag).
    let mut windowed: Vec<tb_reader::Transfer> = by_id
        .into_values()
        .filter(|t| t.timestamp >= cutoff_ts)
        .collect();
    windowed.sort_by_key(|t| t.timestamp);

    Ok(windowed)
}

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
