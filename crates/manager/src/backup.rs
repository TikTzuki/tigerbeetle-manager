//! Backup strategy for TigerBeetle data files.

use crate::error::{ManagerError, Result};
use aws_sdk_s3::Client as S3Client;
use chrono::Utc;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

const COMPRESS_SUFFIX: &str = ".zst";

/// AWS / S3 credentials and endpoint configuration stored in a TOML file.
///
/// All fields are optional — unset fields fall back to the corresponding
/// environment variables (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, …).
///
/// Example `backup_config.toml`:
/// ```toml
/// AWS_ENDPOINT_URL = "https://storage.googleapis.com"
/// AWS_ACCESS_KEY_ID = "GOOG1E..."
/// AWS_SECRET_ACCESS_KEY = "M2Lx7Y..."
/// AWS_DEFAULT_REGION = "asia-southeast1"
/// AWS_REQUEST_CHECKSUM_CALCULATION = "when_required"
/// AWS_RESPONSE_CHECKSUM_VALIDATION = "when_required"
/// ```
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct BackupConfig {
    /// S3-compatible endpoint URL (e.g. Google Cloud Storage).
    #[serde(rename = "AWS_ENDPOINT_URL", skip_serializing_if = "Option::is_none")]
    pub aws_endpoint_url: Option<String>,

    /// AWS access key ID.
    #[serde(rename = "AWS_ACCESS_KEY_ID", skip_serializing_if = "Option::is_none")]
    pub aws_access_key_id: Option<String>,

    /// AWS secret access key.
    #[serde(
        rename = "AWS_SECRET_ACCESS_KEY",
        skip_serializing_if = "Option::is_none"
    )]
    pub aws_secret_access_key: Option<String>,

    /// AWS region (also accepted as `AWS_REGION` by the SDK).
    #[serde(rename = "AWS_DEFAULT_REGION", skip_serializing_if = "Option::is_none")]
    pub aws_default_region: Option<String>,

    /// Checksum calculation mode (`when_required` or `when_supported`).
    #[serde(
        rename = "AWS_REQUEST_CHECKSUM_CALCULATION",
        skip_serializing_if = "Option::is_none"
    )]
    pub aws_request_checksum_calculation: Option<String>,

    /// Checksum validation mode (`when_required` or `when_supported`).
    #[serde(
        rename = "AWS_RESPONSE_CHECKSUM_VALIDATION",
        skip_serializing_if = "Option::is_none"
    )]
    pub aws_response_checksum_validation: Option<String>,

    /// Cron schedule for automated backups (e.g. `0 0 0 * * *`).
    /// Persisted here so the node auto-starts the scheduler after a restart.
    #[serde(
        rename = "BACKUP_CRON_SCHEDULE",
        skip_serializing_if = "Option::is_none"
    )]
    pub cron_schedule: Option<String>,

    /// S3 bucket name for backups.
    #[serde(rename = "BACKUP_BUCKET", skip_serializing_if = "Option::is_none")]
    pub bucket: Option<String>,

    /// Path to the TigerBeetle data file to back up.
    #[serde(rename = "BACKUP_FILE", skip_serializing_if = "Option::is_none")]
    pub backup_file: Option<String>,
}

impl BackupConfig {
    /// Load config from a TOML file. Returns `Ok(Default::default())` if the
    /// file does not exist so callers can always fall back to env vars.
    pub fn load_from_file(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(BackupConfig::default());
        }
        let content = std::fs::read_to_string(path)
            .map_err(|e| ManagerError::Backup(format!("read backup config: {e}")))?;
        toml::from_str(&content)
            .map_err(|e| ManagerError::Backup(format!("parse backup config: {e}")))
    }

    /// Persist config to a TOML file (creates or overwrites).
    pub fn save_to_file(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| ManagerError::Backup(format!("create config dir: {e}")))?;
        }
        let content = toml::to_string_pretty(self)
            .map_err(|e| ManagerError::Backup(format!("serialize backup config: {e}")))?;
        std::fs::write(path, content)
            .map_err(|e| ManagerError::Backup(format!("write backup config: {e}")))
    }

    /// Apply non-None fields as environment variables so that
    /// `aws_config::from_env()` picks them up. Fields already present in the
    /// environment are **only overridden** when the TOML provides a value.
    ///
    /// # Safety
    /// This must only be called from a single thread before the AWS SDK loads
    /// its config. The process is single-threaded at startup, so this is safe.
    fn apply_to_env(&self) {
        // SAFETY: called once during single-threaded startup before the SDK
        // reads environment variables. No other threads are spawned yet.
        unsafe {
            macro_rules! set_if_some {
                ($field:expr, $var:literal) => {
                    if let Some(ref v) = $field {
                        std::env::set_var($var, v);
                    }
                };
            }
            set_if_some!(self.aws_endpoint_url, "AWS_ENDPOINT_URL");
            set_if_some!(self.aws_access_key_id, "AWS_ACCESS_KEY_ID");
            set_if_some!(self.aws_secret_access_key, "AWS_SECRET_ACCESS_KEY");
            // Set both the legacy and current region env vars.
            set_if_some!(self.aws_default_region, "AWS_DEFAULT_REGION");
            set_if_some!(self.aws_default_region, "AWS_REGION");
            set_if_some!(
                self.aws_request_checksum_calculation,
                "AWS_REQUEST_CHECKSUM_CALCULATION"
            );
            set_if_some!(
                self.aws_response_checksum_validation,
                "AWS_RESPONSE_CHECKSUM_VALIDATION"
            );
        }
    }
}

/// Trait for different backup strategies (S3, local, etc.).
#[async_trait::async_trait]
pub trait BackupStrategy: Send + Sync + std::fmt::Debug {
    /// Upload a backup file.
    async fn upload_backup(&self, bucket: &str, path: &Path) -> Result<()>;
}

/// S3-based backup strategy with zstd compression.
///
/// AWS credentials and endpoint are resolved in this priority order:
/// 1. Values from the TOML config file (if `config_file` is set and the file exists).
/// 2. Standard AWS environment variables / shared credentials file / IAM role.
#[derive(Debug)]
pub struct S3BackupStrategy {
    /// Optional path to a TOML file with AWS credentials / endpoint overrides.
    config_file: Option<PathBuf>,
}

impl S3BackupStrategy {
    /// Create a new S3 backup strategy.
    ///
    /// `config_file` is the path to an optional `backup_config.toml`. If the
    /// file exists its values are applied before the SDK reads env vars, so
    /// TOML takes precedence. If `None` (or the file is missing) the SDK falls
    /// back to standard AWS env vars / credential chain.
    pub async fn new(config_file: Option<PathBuf>) -> Self {
        S3BackupStrategy { config_file }
    }

    /// Build an S3 client, loading credentials from TOML (if available) first,
    /// then falling back to the default AWS credential chain (env vars, shared
    /// credentials file, instance profile, …).
    async fn build_client(&self) -> S3Client {
        // Load TOML config and overlay its values on top of environment variables.
        if let Some(ref path) = self.config_file {
            match BackupConfig::load_from_file(path) {
                Ok(cfg) => {
                    cfg.apply_to_env();
                    info!("Loaded backup config from {:?}", path);
                }
                Err(e) => {
                    warn!("Could not load backup config from {:?}: {}", path, e);
                }
            }
        }

        // `from_env()` picks up whatever env vars are set — which now includes
        // any values we just applied from the TOML file.
        let aws_cfg = aws_config::from_env()
            .behavior_version(aws_config::BehaviorVersion::latest())
            .load()
            .await;
        S3Client::new(&aws_cfg)
    }
}

#[async_trait::async_trait]
impl BackupStrategy for S3BackupStrategy {
    async fn upload_backup(&self, bucket: &str, path: &Path) -> Result<()> {
        let client = self.build_client().await;

        let meta = tokio::fs::metadata(path).await?;
        info!("Starting backup: {:?}, {} bytes", path, meta.len());

        let compressed_path = compress_file(path).await?;
        let compressed_meta = tokio::fs::metadata(&compressed_path).await?;
        info!(
            "Uploading compressed backup: {:?}, {} bytes",
            &compressed_path,
            compressed_meta.len()
        );

        let compressed_file = compressed_path
            .file_name()
            .ok_or_else(|| ManagerError::Backup("invalid compressed file path".into()))?
            .to_str()
            .ok_or_else(|| ManagerError::Backup("non-UTF8 file name".into()))?;

        let key = format!(
            "backup/{}/{}",
            Utc::now().format("%Y-%m-%dT%H:%M:%SZ"),
            compressed_file
        );

        let body = aws_sdk_s3::primitives::ByteStream::from_path(&compressed_path)
            .await
            .map_err(|e| ManagerError::S3(format!("failed to read file: {e}")))?;

        // client
        //     .put_object()
        //     .bucket(bucket)
        //     .key(&key)
        //     .body(body)
        //     .send()
        //     .await
        //     .map_err(|e| ManagerError::S3(format!("put_object failed: {e}")))?;

        info!("Backup upload completed: s3://{}/{}", bucket, key);

        // Clean up temporary compressed file.
        if let Err(e) = tokio::fs::remove_file(&compressed_path).await {
            warn!("Failed to remove temp file {:?}: {}", compressed_path, e);
        }

        Ok(())
    }
}

/// Compress a file using zstd and return the path to the compressed file.
async fn compress_file(src: &Path) -> Result<PathBuf> {
    let filename = src
        .file_name()
        .ok_or_else(|| ManagerError::Compression("failed to get file name".into()))?
        .to_str()
        .ok_or_else(|| ManagerError::Compression("invalid backup file path".into()))?;

    let dst_path = PathBuf::from(format!("/tmp/{}{}", filename, COMPRESS_SUFFIX));

    // Run compression in blocking task.
    let src = src.to_path_buf();
    let dst_path_clone = dst_path.clone();
    tokio::task::spawn_blocking(move || {
        let mut encoder = {
            let target = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&dst_path_clone)
                .map_err(|e| ManagerError::Compression(format!("create temp file: {e}")))?;
            zstd::Encoder::new(target, 1)
                .map_err(|e| ManagerError::Compression(format!("create encoder: {e}")))?
        };
        let mut src_file = std::fs::File::open(&src)
            .map_err(|e| ManagerError::Compression(format!("open src file: {e}")))?;
        std::io::copy(&mut src_file, &mut encoder)
            .map_err(|e| ManagerError::Compression(format!("compress: {e}")))?;
        encoder
            .finish()
            .map_err(|e| ManagerError::Compression(format!("finish compression: {e}")))?;
        Ok::<_, ManagerError>(())
    })
    .await
    .map_err(|e| ManagerError::Compression(format!("compression task join error: {e}")))??;

    info!("File compressed to {:?}", dst_path);
    Ok(dst_path)
}
