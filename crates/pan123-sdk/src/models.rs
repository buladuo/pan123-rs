use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenStore {
    pub token: String,
    pub update_time: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CwdStore {
    pub file_id: u64,
    pub path: String,
}

impl Default for CwdStore {
    fn default() -> Self {
        Self {
            file_id: 0,
            path: "/".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiEnvelope<T> {
    #[serde(default)]
    pub code: i64,
    #[serde(default)]
    pub message: String,
    pub data: Option<T>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct DomainData {
    #[serde(default)]
    pub domains: Vec<String>,
    #[serde(rename = "ucenterDomain")]
    pub ucenter_domain: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct UserInfo {
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct FileInfo {
    #[serde(rename = "FileId", alias = "fileId")]
    pub file_id: u64,
    #[serde(rename = "ParentFileId", alias = "parentFileId", default)]
    pub parent_file_id: u64,
    #[serde(rename = "FileName", alias = "fileName", default)]
    pub file_name: String,
    #[serde(rename = "Type", alias = "type", default)]
    pub file_type: u8,
    #[serde(rename = "Size", alias = "size", default)]
    pub size: u64,
    #[serde(rename = "Etag", alias = "etag")]
    pub etag: Option<String>,
    #[serde(rename = "S3KeyFlag", alias = "s3KeyFlag")]
    pub s3_key_flag: Option<String>,
    #[serde(rename = "Status", alias = "status")]
    pub status: Option<i64>,
    #[serde(flatten)]
    pub extra: Value,
}

impl FileInfo {
    pub fn is_dir(&self) -> bool {
        self.file_type != 0
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct FileInfoListData {
    #[serde(
        rename = "infoList",
        default,
        deserialize_with = "deserialize_vec_or_default"
    )]
    pub info_list: Vec<FileInfo>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct FileListData {
    #[serde(
        rename = "InfoList",
        default,
        deserialize_with = "deserialize_vec_or_default"
    )]
    pub info_list: Vec<FileInfo>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct QrGenerateData {
    #[serde(rename = "uniID", default)]
    pub uni_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct QrResultData {
    #[serde(rename = "loginStatus")]
    pub login_status: Option<i64>,
    pub token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct WxCodeData {
    #[serde(rename = "wxCode")]
    pub wx_code: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct SignInData {
    pub token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct CopyTaskCreateData {
    #[serde(rename = "taskId")]
    pub task_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct CopyTaskStatusData {
    pub status: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct TrafficCheckData {
    #[serde(rename = "isTrafficExceeded", default)]
    pub is_traffic_exceeded: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct DispatchItem {
    #[serde(default)]
    pub prefix: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct DownloadInfoData {
    #[serde(rename = "dispatchList", default)]
    pub dispatch_list: Vec<DispatchItem>,
    #[serde(rename = "downloadPath", default)]
    pub download_path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct UploadRequestData {
    #[serde(rename = "Reuse", default)]
    pub reuse: bool,
    #[serde(rename = "Info")]
    pub info: Option<FileInfo>,
    #[serde(rename = "UploadId")]
    pub upload_id: Option<String>,
    #[serde(rename = "Bucket")]
    pub bucket: Option<String>,
    #[serde(rename = "Key")]
    pub key: Option<String>,
    #[serde(rename = "StorageNode")]
    pub storage_node: Option<String>,
    #[serde(
        rename = "FileId",
        deserialize_with = "deserialize_option_u64_from_any"
    )]
    pub file_id: Option<u64>,
    #[serde(
        rename = "SliceSize",
        deserialize_with = "deserialize_option_u64_from_any"
    )]
    pub slice_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct PresignedUrlsData {
    #[serde(rename = "presignedUrls", default)]
    pub presigned_urls: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct UploadCompleteData {
    #[serde(rename = "file_info")]
    pub file_info: Option<FileInfo>,
}

#[derive(Debug, Clone)]
pub struct DownloadLink {
    pub url: String,
    pub filename: String,
}

#[derive(Debug, Clone)]
pub struct DownloadedFile {
    pub file_path: PathBuf,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DownloadResumeMeta {
    pub url: String,
    pub filename: String,
    pub total_bytes: Option<u64>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub downloaded_bytes: u64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DuplicateMode {
    Cancel = 0,
    KeepBoth = 1,
    Overwrite = 2,
}

impl DuplicateMode {
    pub fn as_i64(self) -> i64 {
        self as i64
    }
}

impl fmt::Display for DuplicateMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            DuplicateMode::Cancel => "cancel",
            DuplicateMode::KeepBoth => "keep-both",
            DuplicateMode::Overwrite => "overwrite",
        };
        write!(f, "{value}")
    }
}

#[derive(Debug, Clone)]
pub enum UploadTarget {
    File(PathBuf),
    Directory(PathBuf),
}

#[derive(Debug, Clone)]
pub enum DownloadTarget {
    Single(u64),
    Batch(Vec<u64>),
}

fn deserialize_option_u64_from_any<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_u64()
            .map(Some)
            .ok_or_else(|| serde::de::Error::custom("expected u64-compatible number")),
        Some(Value::String(text)) if text.is_empty() => Ok(None),
        Some(Value::String(text)) => text
            .parse::<u64>()
            .map(Some)
            .map_err(|err| serde::de::Error::custom(err.to_string())),
        Some(other) => Err(serde::de::Error::custom(format!(
            "expected string or number for u64, got {other}"
        ))),
    }
}

fn deserialize_vec_or_default<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let value = Option::<Vec<T>>::deserialize(deserializer)?;
    Ok(value.unwrap_or_default())
}
