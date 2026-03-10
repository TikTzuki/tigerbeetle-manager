//! TigerBeetle manager node — wraps a single TigerBeetle instance with gRPC API.
//!
//! Usage:
//!   tb-manager-node \
//!     --node-id node-0 \
//!     --grpc-port 9090 \
//!     --exe tigerbeetle \
//!     --backup-config-file ./backup_config.toml \
//!     -- start --addresses=3000 ./data/0_0.tigerbeetle
//!
//! Backup settings (cron schedule, bucket, backup file path) are read from
//! the TOML config file and can be updated live via the gRPC API.

use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use tb_manager::{BackupConfig, ManagerConfig, ProcessManager, S3BackupStrategy};
use tigerbeetle_manager_node::proto::manager_node_server::ManagerNodeServer;
use tigerbeetle_manager_node::{ManagerNodeService, proto};
use tokio::sync::watch;
use tonic::transport::Server;
use tracing::{Level, info};
use tracing_subscriber::FmtSubscriber;

#[derive(Parser, Debug)]
#[command(about = "Run a single TigerBeetle manager node with gRPC API")]
struct Args {
    /// Unique node identifier (e.g., "node-0").
    #[arg(long, default_value = "node-0")]
    node_id: String,

    /// Port for gRPC server.
    #[arg(long, default_value_t = 9090)]
    grpc_port: u16,

    /// Path to TigerBeetle executable.
    #[arg(long, default_value = "tigerbeetle")]
    exe: String,

    /// Path to TOML file with backup settings and AWS/S3 credentials.
    /// Stores: BACKUP_FILE, BACKUP_BUCKET, BACKUP_CRON_SCHEDULE, AWS_* credentials.
    /// All backup settings are read from here and can be updated via the gRPC API.
    #[arg(long)]
    backup_config_file: Option<PathBuf>,

    /// Arguments to pass to TigerBeetle (after --).
    #[arg(last = true)]
    child_args: Vec<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    let args = Args::parse();

    // Load TOML config — all backup settings live here.
    let toml_cfg: BackupConfig = if let Some(ref path) = args.backup_config_file {
        match BackupConfig::load_from_file(path) {
            Ok(cfg) => cfg,
            Err(e) => {
                tracing::warn!("Could not load backup config on startup: {}", e);
                BackupConfig::default()
            }
        }
    } else {
        BackupConfig::default()
    };

    let initial_cron = toml_cfg.cron_schedule.clone();
    let bucket = toml_cfg
        .bucket
        .clone()
        .unwrap_or_else(|| "tigerbeetle-backups".into());
    let backup_file = PathBuf::from(
        toml_cfg
            .backup_file
            .clone()
            // Fall back to the data file passed to TigerBeetle (last non-flag arg),
            // so ReadAccounts/ReadTransfers work even without a backup config.
            .or_else(|| {
                args.child_args
                    .iter()
                    .rev()
                    .find(|a| !a.starts_with("--"))
                    .cloned()
            })
            .unwrap_or_else(|| "./data/0_0.tigerbeetle".into()),
    );

    info!("TigerBeetle Manager Node '{}' starting", args.node_id);
    info!("  gRPC port:        {}", args.grpc_port);
    info!("  Executable:       {}", args.exe);
    info!("  Args:             {:?}", args.child_args);
    info!("  Backup file:      {:?}", backup_file);
    info!("  S3 bucket:        {}", bucket);
    info!(
        "  Cron schedule:    {}",
        initial_cron.as_deref().unwrap_or("<disabled>")
    );
    info!(
        "  Config file:      {}",
        args.backup_config_file
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<not set>".into())
    );

    // Watch channel for live cron-schedule changes.
    let (cron_tx, cron_rx) = watch::channel(initial_cron);
    let cron_tx = Arc::new(cron_tx);

    let config = ManagerConfig {
        exe: args.exe,
        args: args.child_args,
        backup_file,
        bucket,
    };

    let backup_strategy = S3BackupStrategy::new(args.backup_config_file.clone()).await;

    // Log broadcast channel for streaming logs to gRPC clients.
    let (log_tx, _) = tokio::sync::broadcast::channel::<proto::LogEntry>(1024);

    // Convert manager LogEntry to proto LogEntry for the broadcast channel.
    let (manager_log_tx, mut manager_log_rx) =
        tokio::sync::broadcast::channel::<tb_manager::LogEntry>(1024);

    let proto_log_tx = log_tx.clone();
    tokio::spawn(async move {
        loop {
            match manager_log_rx.recv().await {
                Ok(entry) => {
                    let proto_entry = proto::LogEntry {
                        timestamp: entry.timestamp,
                        level: match entry.level {
                            tb_manager::LogLevel::Info => proto::LogLevel::Info.into(),
                            tb_manager::LogLevel::Warn => proto::LogLevel::Warn.into(),
                            tb_manager::LogLevel::Error => proto::LogLevel::Error.into(),
                        },
                        message: entry.message,
                    };
                    let _ = proto_log_tx.send(proto_entry);
                }
                Err(_) => break,
            }
        }
    });

    let manager = ProcessManager::new(config, backup_strategy, Some(manager_log_tx), cron_rx);

    let node_state = tigerbeetle_manager_node::grpc_service::NodeState {
        node_id: args.node_id.clone(),
        manager_state: manager.manager_state.clone(),
        log_tx,
        started_at: chrono::Utc::now(),
        backup_config_file: args.backup_config_file,
        backup_lock: Arc::new(tokio::sync::Mutex::new(())),
        cron_schedule_tx: cron_tx,
    };
    let grpc_service = ManagerNodeService::new(node_state);

    let grpc_addr = format!("0.0.0.0:{}", args.grpc_port).parse()?;
    info!("gRPC server listening on {}", grpc_addr);

    tokio::spawn(async move {
        if let Err(e) = Server::builder()
            .accept_http1(true)
            .add_service(tonic_web::enable(ManagerNodeServer::new(grpc_service)))
            .serve(grpc_addr)
            .await
        {
            tracing::error!("gRPC server error: {}", e);
        }
    });

    manager.run().await?;

    Ok(())
}
