use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::models::FileInfo;

#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    pub max_attempts: usize,
    pub base_delay_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay_ms: 750,
        }
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
            parallelism: std::thread::available_parallelism()
                .map(|n| n.get().min(4))
                .unwrap_or(3),
            retry: RetryPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct UploadOptions {
    pub transfer: TransferOptions,
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
