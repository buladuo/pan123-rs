use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::models::FileInfo;

#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    pub max_attempts: usize,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
    pub exponential_backoff: bool,
    pub jitter: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay_ms: 750,
            max_delay_ms: 30_000,
            exponential_backoff: true,
            jitter: true,
        }
    }
}

impl RetryPolicy {
    pub fn aggressive() -> Self {
        Self {
            max_attempts: 5,
            base_delay_ms: 500,
            max_delay_ms: 60_000,
            exponential_backoff: true,
            jitter: true,
        }
    }

    pub fn very_aggressive() -> Self {
        Self {
            max_attempts: 10,
            base_delay_ms: 1000,
            max_delay_ms: 120_000,
            exponential_backoff: true,
            jitter: true,
        }
    }

    pub fn conservative() -> Self {
        Self {
            max_attempts: 2,
            base_delay_ms: 1000,
            max_delay_ms: 10_000,
            exponential_backoff: false,
            jitter: false,
        }
    }

    pub fn calculate_delay(&self, attempt: usize) -> u64 {
        let mut delay = if self.exponential_backoff {
            self.base_delay_ms
                .saturating_mul(2u64.saturating_pow(attempt.saturating_sub(1) as u32))
        } else {
            self.base_delay_ms.saturating_mul(attempt as u64)
        };

        delay = delay.min(self.max_delay_ms);

        if self.jitter {
            use rand::Rng;
            let jitter_range = (delay as f64 * 0.2) as u64;
            let jitter = rand::rng().random_range(0..=jitter_range);
            delay = delay.saturating_add(jitter);
        }

        delay
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TransferOptions {
    pub parallelism: usize,
    pub retry: RetryPolicy,
}

impl Default for TransferOptions {
    fn default() -> Self {
        Self {
            parallelism: 1, // Web端不支持并发，使用顺序上传
            retry: RetryPolicy::default(),
        }
    }
}

/// 上传选项。
#[derive(Debug, Clone, Copy)]
pub struct UploadOptions {
    pub transfer: TransferOptions,
    /// 是否启用「跨会话断点续传」。
    ///
    /// 为 `true`（默认）时，上传前会查找本地记录的未完成会话
    /// （`<config_dir>/upload_sessions/*.json`，键形如
    /// `mfy-upload-session:{md5}:{parent}:{name}:{size}`），命中则复用其
    /// `uploadId`，并在服务端响应字段缺失时用会话兜底。
    ///
    /// 设为 `false` 时**不读取**本地会话，等价于「不信任上次的记录，重新走一遍
    /// `upload_request`」。注意：这**不会**禁用「跳过服务端已存在的分片」——
    /// 那由 `file/s3_list_upload_parts` 决定，属协议固有行为（同一个文件身份
    /// 服务端本就返回同一个 `uploadId`，重复传分片没有意义）。
    pub resume: bool,
}

impl Default for UploadOptions {
    fn default() -> Self {
        Self {
            transfer: TransferOptions::default(),
            resume: true,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DownloadOptions {
    pub transfer: TransferOptions,
    pub resume: bool,
}

impl DownloadOptions {
    pub fn normalized(mut self) -> Self {
        if !self.resume {
            self.resume = true;
        }
        self
    }
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            transfer: TransferOptions::default(),
            resume: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum TransferKind {
    Upload,
    Download,
}

#[derive(Debug, Clone)]
pub enum TransferEvent {
    Started {
        id: String,
        kind: TransferKind,
        path: PathBuf,
        total_bytes: Option<u64>,
    },
    Progress {
        id: String,
        kind: TransferKind,
        bytes: u64,
        total_bytes: Option<u64>,
    },
    Retrying {
        id: String,
        kind: TransferKind,
        attempt: usize,
        message: String,
    },
    Finished {
        id: String,
        kind: TransferKind,
        path: PathBuf,
        total_bytes: u64,
    },
    Failed {
        id: String,
        kind: TransferKind,
        path: PathBuf,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum UploadFailureKind {
    LocalIo,
    Network,
    RemoteApi,
    Conflict,
    Validation,
    Auth,
    Unknown,
}

impl UploadFailureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            UploadFailureKind::LocalIo => "local-io",
            UploadFailureKind::Network => "network",
            UploadFailureKind::RemoteApi => "remote-api",
            UploadFailureKind::Conflict => "conflict",
            UploadFailureKind::Validation => "validation",
            UploadFailureKind::Auth => "auth",
            UploadFailureKind::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TransferFailure {
    pub path: PathBuf,
    pub kind: UploadFailureKind,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct UploadDirectoryReport {
    pub uploaded: Vec<FileInfo>,
    pub failed: Vec<TransferFailure>,
}

impl UploadDirectoryReport {
    pub fn is_complete_success(&self) -> bool {
        self.failed.is_empty()
    }

    pub fn uploaded_count(&self) -> usize {
        self.uploaded.len()
    }

    pub fn failed_count(&self) -> usize {
        self.failed.len()
    }

    pub fn failure_counts(&self) -> BTreeMap<UploadFailureKind, usize> {
        let mut counts = BTreeMap::new();
        for failure in &self.failed {
            *counts.entry(failure.kind).or_insert(0) += 1;
        }
        counts
    }
}

pub type ProgressCallback = Arc<dyn Fn(TransferEvent) + Send + Sync + 'static>;

/// 传输取消令牌。`true` 表示已被取消。
///
/// 由调用方创建并持有；SDK 在传输循环中检查它，一旦置位即返回
/// [`crate::Pan123Error::Cancelled`]。CLI 与 MCP 之外的调用方传 `None` 即可，
/// 行为与加入该特性之前完全一致。
pub type CancelToken = Arc<AtomicBool>;

/// 便于在传输循环里做检查点。
pub trait CancelTokenExt {
    fn check(&self) -> crate::Result<()>;
}

impl CancelTokenExt for CancelToken {
    fn check(&self) -> crate::Result<()> {
        if self.load(Ordering::SeqCst) {
            Err(crate::Pan123Error::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// 可取消的流拷贝。`on_chunk` 每次读取后调用，参数为累计写入字节数。
pub fn copy_stream_cancellable<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    cancel: Option<&CancelToken>,
    mut on_chunk: impl FnMut(u64),
) -> crate::Result<u64> {
    let mut buffer = vec![0u8; 64 * 1024];
    let mut written: u64 = 0;
    loop {
        if let Some(token) = cancel {
            token.check()?;
        }
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        writer.write_all(&buffer[..read])?;
        written += read as u64;
        on_chunk(written);
    }
    Ok(written)
}

#[cfg(test)]
mod cancel_tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::atomic::Ordering;

    #[test]
    fn copies_all_bytes_when_not_cancelled() {
        let mut reader = Cursor::new(vec![7u8; 4096]);
        let mut writer = Vec::new();
        let mut seen = Vec::new();
        let written =
            copy_stream_cancellable(&mut reader, &mut writer, None, |total| seen.push(total))
                .expect("copy should succeed");
        assert_eq!(written, 4096);
        assert_eq!(writer.len(), 4096);
        assert_eq!(seen.last().copied(), Some(4096));
    }

    #[test]
    fn returns_cancelled_when_token_already_set() {
        let token: CancelToken = Arc::new(AtomicBool::new(true));
        let mut reader = Cursor::new(vec![7u8; 4096]);
        let mut writer = Vec::new();
        let err = copy_stream_cancellable(&mut reader, &mut writer, Some(&token), |_| {})
            .expect_err("should be cancelled");
        assert!(matches!(err, crate::Pan123Error::Cancelled));
        assert_eq!(writer.len(), 0, "取消后不应写入任何数据");
    }

    #[test]
    fn stops_midway_when_cancelled_during_copy() {
        // 输入 256 KiB、内部缓冲 64 KiB：第一块读满后回调置位取消令牌，
        // 第二块开始前必须检出并停止，因此写入量应远小于总和。
        let token: CancelToken = Arc::new(AtomicBool::new(false));
        let token_for_cb = Arc::clone(&token);
        let mut reader = Cursor::new(vec![7u8; 256 * 1024]);
        let mut writer = Vec::new();
        let err = copy_stream_cancellable(&mut reader, &mut writer, Some(&token), move |total| {
            if total >= 1024 {
                token_for_cb.store(true, Ordering::SeqCst);
            }
        })
        .expect_err("should be cancelled midway");
        assert!(matches!(err, crate::Pan123Error::Cancelled));
        assert!(
            writer.len() < 256 * 1024,
            "应提前停止，实际写入 {}",
            writer.len()
        );
        assert!(!writer.is_empty(), "取消前应至少完成一个分块");
    }
}
