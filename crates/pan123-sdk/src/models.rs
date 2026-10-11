use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::fmt;
use std::path::PathBuf;
use url::Url;

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
    /// 新版接口直接下发扫码地址；旧版可能缺失。
    #[serde(default)]
    pub url: Option<String>,
}

impl QrGenerateData {
    /// 构造真正可扫的二维码内容。
    ///
    /// `qr-code/generate` 返回的 `url` 是一个**不带任何查询参数的基址**
    /// （实测为 `https://yun.123pan.cn/wx-app-login.html`），`uniID` 是独立字段。
    /// H5 页面靠 `location.search` 读取 `uniID`，缺失时页面上的
    /// `POST /api/user/qr-code/scan` 会返回 `uniID为必填字段`（code -1），
    /// 于是 App/微信扫码后没有任何反应。若把接口下发的 `url` 原样编码成二维码，
    /// 就等于扫了一个「什么都不会发生」的网页。
    ///
    /// 因此这里把 `uniID`（以及页面用来选环境的 `env`）合并进 `url`。
    /// 若 `url` 已经带了 `uniID`（服务端未来改成直接下发完整地址），则原样返回。
    pub fn scan_url(&self) -> Option<String> {
        let base = self
            .url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())?;

        // 服务端已下发完整地址时不做任何改写。
        if let Ok(parsed) = Url::parse(base)
            && parsed
                .query_pairs()
                .any(|(key, _)| key.eq_ignore_ascii_case("uniID"))
        {
            return Some(base.to_string());
        }

        let mut parsed = Url::parse(base).ok()?;
        parsed.query_pairs_mut().append_pair("env", "production");
        parsed.query_pairs_mut().append_pair("uniID", &self.uni_id);
        Some(parsed.to_string())
    }
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
    /// 服务端上传状态；255 表示需走 `upload_complete/result` 异步轮询。
    #[serde(
        rename = "UploadFileStatus",
        deserialize_with = "deserialize_option_u64_from_any"
    )]
    pub upload_file_status: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct PresignedUrlsData {
    /// 键为 **1 起始** 的分片序号，值为该分片的 PUT 预签名地址。
    #[serde(rename = "presignedUrls", default)]
    pub presigned_urls: std::collections::HashMap<String, String>,
    /// 部分存储节点要求上传分片时一并携带的附加请求头（web 端会原样透传到 PUT）。
    #[serde(default)]
    pub headers: std::collections::HashMap<String, String>,
}

/// `file/s3_list_upload_parts` 返回的单个「已存在于对象存储上的分片」。
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct UploadedPart {
    #[serde(
        rename = "PartNumber",
        default,
        deserialize_with = "deserialize_option_u64_from_any"
    )]
    pub part_number: Option<u64>,
    #[serde(rename = "ETag", default)]
    pub etag: Option<String>,
    #[serde(
        rename = "Size",
        default,
        deserialize_with = "deserialize_option_u64_from_any"
    )]
    pub size: Option<u64>,
}

/// `file/s3_list_upload_parts` 响应体。
///
/// 后端不同版本分别使用 `Parts` / `parts` 作为字段名，这里两者都接受。
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ListPartsData {
    #[serde(rename = "Parts", alias = "parts", default)]
    pub parts: Vec<UploadedPart>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct UploadCompleteData {
    #[serde(rename = "file_info", alias = "FileInfo", alias = "Info")]
    pub file_info: Option<FileInfo>,
    #[serde(rename = "async")]
    pub is_async: Option<bool>,
    /// `upload_complete/result` 轮询建议的等待秒数。
    #[serde(default, deserialize_with = "deserialize_option_u64_from_any")]
    pub duration: Option<u64>,
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

#[cfg(test)]
mod qr_scan_url_tests {
    use super::QrGenerateData;

    fn parse(data: &str) -> QrGenerateData {
        serde_json::from_str(data).expect("deserialize QrGenerateData")
    }

    #[test]
    fn appends_uniid_to_bare_url_from_api() {
        // 真实抓包响应：url 是无查询参数的基址，uniID 独立下发。
        let data = parse(
            r#"{"url":"https://yun.123pan.cn/wx-app-login.html","uniID":"f49a77ee-0a4d-4ca9-a359-b7d27f9182c7"}"#,
        );
        let url = data.scan_url().expect("scan url");
        let parsed = url::Url::parse(&url).expect("valid url");
        let pairs: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(
            pairs.get("uniID").map(String::as_str),
            Some(data.uni_id.as_str())
        );
        assert_eq!(pairs.get("env").map(String::as_str), Some("production"));
        assert_eq!(parsed.path(), "/wx-app-login.html");
    }

    #[test]
    fn keeps_url_untouched_when_uniid_already_present() {
        let raw = "https://www.123pan.com/wx-app-login.html?env=production&uniID=abc&type=login";
        let data = parse(&format!(r#"{{"url":"{raw}","uniID":"abc"}}"#));
        assert_eq!(data.scan_url().as_deref(), Some(raw));
    }

    #[test]
    fn preserves_existing_query_params() {
        let data = parse(
            r#"{"url":"https://www.123pan.com/wx-app-login.html?source=123pan","uniID":"abc"}"#,
        );
        let url = data.scan_url().expect("scan url");
        let parsed = url::Url::parse(&url).expect("valid url");
        assert!(
            parsed
                .query_pairs()
                .any(|(key, value)| key == "source" && value == "123pan")
        );
        assert!(
            parsed
                .query_pairs()
                .any(|(k, v)| k == "uniID" && v == "abc")
        );
    }

    #[test]
    fn returns_none_when_url_absent_or_blank() {
        let missing = parse(r#"{"uniID":"abc"}"#);
        assert!(missing.scan_url().is_none());
        let blank = parse(r#"{"url":"   ","uniID":"abc"}"#);
        assert!(blank.scan_url().is_none());
    }

    #[test]
    fn deserializes_legacy_payload_without_url() {
        let data = parse(r#"{"uniID":"abc"}"#);
        assert_eq!(data.uni_id, "abc");
        assert!(data.url.is_none());
    }
}
