//! 上传会话的本地持久化，用于「上传失败 / 进程中断后的跨会话自动恢复」。
//!
//! 契约来源：web 前端上传模块（`mod_10796`）的 sessionStorage 实现。
//!
//! * 存储键：`mfy-upload-session:{fileMd5}:{parentFileId}:{fileName}:{fileSize}`
//! * 存储体：
//!   `{uploadId, key, bucket, storageNode, sliceSize, cloudFileId, uploadFileStatus,
//!     fileMd5, fileName, fileSize, parentFileId, savedAt}`
//! * 有效期：7 天（前端常量 `6048e5` 毫秒），过期即失效并从磁盘清理
//! * 有效性：必须同时具备非空的 `uploadId` / `key` / `bucket`（前端 `m()`）
//!
//! 与前端有两处**有意为之**的差异：
//!
//! 1. 前端只在「用户主动取消」时写入 sessionStorage；这里在拿到 `uploadId` 后
//!    **立即落盘**。因为进程崩溃、断电、`Ctrl-C` 都不会执行取消回调，
//!    而跨会话恢复恰恰要覆盖这类「非优雅退出」的情况。
//! 2. 前端的宽松查找只比对 `fileMd5`；这里额外要求 `parentFileId` 一致，
//!    避免把 A 目录的会话错用到 B 目录。
//!
//! 磁盘布局：`<config_dir>/upload_sessions/<md5(session_key)>.json`。
//! 用键的摘要作为文件名，是因为键里含 `:` 且文件名本身可能包含 `/` 等非法字符。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config;
use crate::error::Result;
use crate::models::UploadRequestData;

/// 存储键前缀，与 web 端 `A` 常量完全一致。
pub const SESSION_KEY_PREFIX: &str = "mfy-upload-session:";

/// 会话有效期：7 天。对齐 web 端 `6048e5`（毫秒）。
pub const SESSION_TTL_MILLIS: i64 = 604_800_000;

/// 一条未完成上传的会话记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadSession {
    #[serde(rename = "uploadId")]
    pub upload_id: String,
    pub key: String,
    pub bucket: String,
    #[serde(
        rename = "storageNode",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub storage_node: Option<String>,
    #[serde(rename = "sliceSize", default, skip_serializing_if = "Option::is_none")]
    pub slice_size: Option<u64>,
    #[serde(
        rename = "cloudFileId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub cloud_file_id: Option<u64>,
    #[serde(
        rename = "uploadFileStatus",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub upload_file_status: Option<u64>,
    #[serde(rename = "fileMd5")]
    pub file_md5: String,
    #[serde(rename = "fileName")]
    pub file_name: String,
    #[serde(rename = "fileSize")]
    pub file_size: u64,
    #[serde(rename = "parentFileId")]
    pub parent_file_id: u64,
    /// 写入时刻（epoch 毫秒），与前端 `Date.now()` 对齐。
    #[serde(rename = "savedAt")]
    pub saved_at: i64,
}

impl UploadSession {
    /// 组装存储键，格式与前端 `I()` 一致。
    pub fn storage_key(&self) -> String {
        build_key(
            &self.file_md5,
            self.parent_file_id,
            &self.file_name,
            self.file_size,
        )
    }

    /// 与前端 `m()` 的有效性判定一致：`uploadId` / `key` / `bucket` 缺一不可。
    pub fn is_valid(&self) -> bool {
        !self.upload_id.trim().is_empty()
            && !self.key.trim().is_empty()
            && !self.bucket.trim().is_empty()
    }

    /// 是否已超过 7 天有效期。
    pub fn is_expired_at(&self, now_millis: i64) -> bool {
        now_millis.saturating_sub(self.saved_at) > SESSION_TTL_MILLIS
    }

    /// 该会话是否可用于恢复指定文件的本次上传。
    ///
    /// 要求 md5 与父目录一致；文件名 / 大小若已记录则必须一致。
    pub fn matches(&self, md5: &str, parent_id: u64, file_name: &str, file_size: u64) -> bool {
        self.file_md5 == md5
            && self.parent_file_id == parent_id
            && self.file_name == file_name
            && self.file_size == file_size
    }

    /// 用 `upload_request` 的响应构造一条会话记录。
    ///
    /// 缺少 `uploadId` / `key` / `bucket` 时返回 `None`（无会话可持久化）。
    pub fn from_request(
        data: &UploadRequestData,
        md5: &str,
        file_name: &str,
        file_size: u64,
        parent_id: u64,
    ) -> Option<Self> {
        let session = Self {
            upload_id: data.upload_id.clone()?,
            key: data.key.clone()?,
            bucket: data.bucket.clone()?,
            storage_node: data.storage_node.clone(),
            slice_size: data.slice_size,
            cloud_file_id: data.file_id,
            upload_file_status: data.upload_file_status,
            file_md5: md5.to_string(),
            file_name: file_name.to_string(),
            file_size,
            parent_file_id: parent_id,
            saved_at: now_millis(),
        };
        session.is_valid().then_some(session)
    }
}

/// 把会话里记录的信息回填到 `upload_request` 的响应上。
///
/// 对应前端 `b()`：服务端未下发的字段用本地会话兜底，
/// 保证即使响应体不完整（缺 `UploadId` / `Bucket` / `StorageNode` 等）也能继续续传。
pub fn apply_fallback(data: &mut UploadRequestData, saved: &UploadSession) {
    if data.upload_id.as_deref().unwrap_or("").is_empty() {
        data.upload_id = Some(saved.upload_id.clone());
    }
    if data.key.as_deref().unwrap_or("").is_empty() {
        data.key = Some(saved.key.clone());
    }
    if data.bucket.as_deref().unwrap_or("").is_empty() {
        data.bucket = Some(saved.bucket.clone());
    }
    if data.storage_node.is_none() {
        data.storage_node = saved.storage_node.clone();
    }
    if data.slice_size.is_none() {
        data.slice_size = saved.slice_size;
    }
    if data.file_id.is_none() {
        data.file_id = saved.cloud_file_id;
    }
    if data.upload_file_status.is_none() {
        data.upload_file_status = saved.upload_file_status;
    }
}

/// 当前 Unix 时间戳（毫秒）。与前端 `Date.now()` 语义一致。
pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

/// 组装存储键字符串。
pub fn build_key(md5: &str, parent_id: u64, file_name: &str, file_size: u64) -> String {
    format!("{SESSION_KEY_PREFIX}{md5}:{parent_id}:{file_name}:{file_size}")
}

/// 写入一条会话记录（覆盖同键的旧记录）。
pub fn save(session: &UploadSession) -> Result<()> {
    save_in(&config::upload_session_dir(), session)
}

/// 按「md5 + 父目录 + 文件名 + 大小」精确查找会话。
pub fn load_exact(
    md5: &str,
    parent_id: u64,
    file_name: &str,
    file_size: u64,
) -> Option<UploadSession> {
    load_exact_in(
        &config::upload_session_dir(),
        md5,
        parent_id,
        file_name,
        file_size,
    )
}

/// 查找可用于本次上传的会话。
///
/// 先精确匹配（md5 + 父目录 + 名称 + 大小）；未命中时退化为「md5 + 父目录」的宽松匹配，
/// 以覆盖文件名被改动等边缘情况。
pub fn load_for_file(
    md5: &str,
    parent_id: u64,
    file_name: &str,
    file_size: u64,
) -> Option<UploadSession> {
    load_for_file_in(
        &config::upload_session_dir(),
        md5,
        parent_id,
        file_name,
        file_size,
    )
}

/// 列出所有**仍然有效**的会话，按 `savedAt` 降序（最近的在最前）。
/// 顺带清理已过期的记录。
pub fn list() -> Vec<UploadSession> {
    list_in(&config::upload_session_dir())
}

/// 删除与给定文件同 md5（且名称 / 大小一致）的所有会话记录，返回删除数量。
///
/// 对应前端 `v()`：上传成功后清掉残留，避免下次误用旧 uploadId。
pub fn remove_stale(md5: &str, file_name: &str, file_size: u64) -> usize {
    remove_stale_in(&config::upload_session_dir(), md5, file_name, file_size)
}

/// 清空全部会话记录，返回删除数量。
pub fn clear() -> Result<usize> {
    clear_in(&config::upload_session_dir())
}

/// 清理已过期会话，返回删除数量。
pub fn purge_expired() -> usize {
    purge_expired_in(&config::upload_session_dir())
}

// ---------------------------------------------------------------------------
// 内部实现：全部以显式 root 目录为参数，便于单元测试使用临时目录。
// ---------------------------------------------------------------------------

fn path_for_key_in(root: &Path, key: &str) -> PathBuf {
    let digest = format!("{:x}", md5::compute(key.as_bytes()));
    root.join(format!("{digest}.json"))
}

fn save_in(root: &Path, session: &UploadSession) -> Result<()> {
    if !session.is_valid() {
        return Ok(());
    }
    fs::create_dir_all(root)?;
    let path = path_for_key_in(root, &session.storage_key());
    fs::write(path, serde_json::to_string_pretty(session)?)?;
    Ok(())
}

fn read_session(path: &Path) -> Option<UploadSession> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str::<UploadSession>(&text).ok()
}

/// 读取并做有效性 + TTL 校验；不合法/过期时顺手删除文件。
fn read_valid(path: &Path, now_millis: i64) -> Option<UploadSession> {
    let session = read_session(path)?;
    if !session.is_valid() || session.is_expired_at(now_millis) {
        let _ = fs::remove_file(path);
        return None;
    }
    Some(session)
}

fn load_exact_in(
    root: &Path,
    md5: &str,
    parent_id: u64,
    file_name: &str,
    file_size: u64,
) -> Option<UploadSession> {
    let key = build_key(md5, parent_id, file_name, file_size);
    let path = path_for_key_in(root, &key);
    let session = read_valid(&path, now_millis())?;
    session
        .matches(md5, parent_id, file_name, file_size)
        .then_some(session)
}

fn load_for_file_in(
    root: &Path,
    md5: &str,
    parent_id: u64,
    file_name: &str,
    file_size: u64,
) -> Option<UploadSession> {
    if let Some(session) = load_exact_in(root, md5, parent_id, file_name, file_size) {
        return Some(session);
    }
    // 宽松回退：md5 + 父目录一致即可复用，容忍文件名/大小记录的差异。
    list_in(root)
        .into_iter()
        .find(|session| session.file_md5 == md5 && session.parent_file_id == parent_id)
}

fn list_in(root: &Path) -> Vec<UploadSession> {
    let mut sessions = Vec::new();
    let Ok(entries) = fs::read_dir(root) else {
        return sessions;
    };
    let now = now_millis();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        if let Some(session) = read_valid(&path, now) {
            sessions.push(session);
        }
    }
    sessions.sort_by(|left, right| right.saved_at.cmp(&left.saved_at));
    sessions
}

fn remove_stale_in(root: &Path, md5: &str, file_name: &str, file_size: u64) -> usize {
    let mut removed = 0usize;
    let Ok(entries) = fs::read_dir(root) else {
        return removed;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let Some(session) = read_session(&path) else {
            continue;
        };
        let name_matches = file_name.is_empty() || session.file_name == file_name;
        let size_matches = file_size == 0 || session.file_size == file_size;
        if session.file_md5 == md5 && name_matches && size_matches {
            let _ = fs::remove_file(&path);
            removed += 1;
        }
    }
    removed
}

fn clear_in(root: &Path) -> Result<usize> {
    let mut removed = 0usize;
    let Ok(entries) = fs::read_dir(root) else {
        return Ok(removed);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            fs::remove_file(&path)?;
            removed += 1;
        }
    }
    Ok(removed)
}

fn purge_expired_in(root: &Path) -> usize {
    let mut removed = 0usize;
    let Ok(entries) = fs::read_dir(root) else {
        return removed;
    };
    let now = now_millis();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        match read_session(&path) {
            Some(session) if !session.is_valid() || session.is_expired_at(now) => {
                if fs::remove_file(&path).is_ok() {
                    removed += 1;
                }
            }
            None => {
                // 损坏的记录一并清掉。
                if fs::remove_file(&path).is_ok() {
                    removed += 1;
                }
            }
            _ => {}
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> UploadSession {
        UploadSession {
            upload_id: "upload-abc".into(),
            key: "drive/2026/abc".into(),
            bucket: "pan123-bucket".into(),
            storage_node: Some("node-1".into()),
            slice_size: Some(16 * 1024 * 1024),
            cloud_file_id: Some(42),
            upload_file_status: Some(0),
            file_md5: "d41d8cd98f00b204e9800998ecf8427e".into(),
            file_name: "demo.bin".into(),
            file_size: 64 * 1024 * 1024,
            parent_file_id: 41981424,
            saved_at: now_millis(),
        }
    }

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "pan123-upload-session-{tag}-{}",
            uuid::Uuid::new_v4()
        ));
        let _ = fs::create_dir_all(&root);
        root
    }

    #[test]
    fn storage_key_matches_frontend_format() {
        let session = sample();
        assert_eq!(
            session.storage_key(),
            "mfy-upload-session:d41d8cd98f00b204e9800998ecf8427e:41981424:demo.bin:67108864"
        );
        assert!(session.storage_key().starts_with(SESSION_KEY_PREFIX));
    }

    #[test]
    fn validity_requires_id_key_bucket() {
        let mut session = sample();
        assert!(session.is_valid());

        session.upload_id = String::new();
        assert!(!session.is_valid());

        session.upload_id = "x".into();
        session.bucket = "  ".into();
        assert!(!session.is_valid());
    }

    #[test]
    fn ttl_is_seven_days() {
        let mut session = sample();
        let now = now_millis();

        session.saved_at = now - 6 * 24 * 60 * 60 * 1000;
        assert!(!session.is_expired_at(now), "6 天应仍然有效");

        session.saved_at = now - 8 * 24 * 60 * 60 * 1000;
        assert!(session.is_expired_at(now), "8 天应判定过期");
    }

    #[test]
    fn serialized_shape_is_frontend_compatible() {
        let value = serde_json::to_value(sample()).unwrap();
        for field in [
            "uploadId",
            "key",
            "bucket",
            "storageNode",
            "sliceSize",
            "cloudFileId",
            "uploadFileStatus",
            "fileMd5",
            "fileName",
            "fileSize",
            "parentFileId",
            "savedAt",
        ] {
            assert!(value.get(field).is_some(), "缺少字段 {field}");
        }
    }

    #[test]
    fn save_then_load_roundtrip() {
        let root = temp_root("roundtrip");
        let session = sample();
        save_in(&root, &session).unwrap();

        let loaded = load_exact_in(
            &root,
            &session.file_md5,
            41981424,
            "demo.bin",
            64 * 1024 * 1024,
        )
        .expect("应能按精确键找回会话");
        assert_eq!(loaded.upload_id, "upload-abc");
        assert_eq!(loaded.bucket, "pan123-bucket");

        // 父目录不同 -> 不应命中。
        assert!(load_exact_in(&root, &session.file_md5, 1, "demo.bin", 64 * 1024 * 1024).is_none());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn loose_lookup_tolerates_name_change() {
        let root = temp_root("loose");
        save_in(&root, &sample()).unwrap();

        // 文件名变了，但 md5 + 父目录一致 -> 宽松查找应命中。
        let found = load_for_file_in(
            &root,
            &sample().file_md5,
            41981424,
            "renamed.bin",
            64 * 1024 * 1024,
        );
        assert!(found.is_some(), "宽松查找应容忍文件名变化");

        // 父目录不同 -> 即便 md5 一致也不应命中。
        assert!(
            load_for_file_in(&root, &sample().file_md5, 999, "demo.bin", 64 * 1024 * 1024)
                .is_none()
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn expired_session_is_rejected_and_purged() {
        let root = temp_root("expired");
        let mut session = sample();
        session.saved_at = now_millis() - 8 * 24 * 60 * 60 * 1000;
        save_in(&root, &session).unwrap();

        assert!(list_in(&root).is_empty(), "过期会话不应出现在列表中");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0, "过期文件应被清理");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn invalid_session_is_rejected() {
        let root = temp_root("invalid");
        let mut session = sample();
        session.upload_id = String::new();
        // 绕过 save_in 的有效性校验，直接落盘，模拟历史脏数据。
        fs::write(
            path_for_key_in(&root, &session.storage_key()),
            serde_json::to_string(&session).unwrap(),
        )
        .unwrap();

        assert!(
            load_exact_in(
                &root,
                &session.file_md5,
                41981424,
                "demo.bin",
                64 * 1024 * 1024
            )
            .is_none()
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn remove_stale_clears_matching_records() {
        let root = temp_root("stale");
        save_in(&root, &sample()).unwrap();
        assert_eq!(list_in(&root).len(), 1);

        assert_eq!(
            remove_stale_in(&root, &sample().file_md5, "demo.bin", 64 * 1024 * 1024),
            1
        );
        assert!(list_in(&root).is_empty());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn clear_removes_everything() {
        let root = temp_root("clear");
        save_in(&root, &sample()).unwrap();
        let mut other = sample();
        other.file_md5 = "ffffffffffffffffffffffffffffffff".into();
        other.file_name = "other.bin".into();
        save_in(&root, &other).unwrap();

        assert_eq!(clear_in(&root).unwrap(), 2);
        assert!(list_in(&root).is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn apply_fallback_fills_missing_fields_only() {
        let saved = sample();
        let mut data = UploadRequestData {
            reuse: false,
            info: None,
            upload_id: None,
            bucket: Some("server-bucket".into()),
            key: None,
            storage_node: None,
            file_id: None,
            slice_size: None,
            upload_file_status: None,
        };
        apply_fallback(&mut data, &saved);

        // 服务端已给出的字段必须保留，不能被本地会话覆盖。
        assert_eq!(data.bucket.as_deref(), Some("server-bucket"));
        assert_eq!(data.upload_id.as_deref(), Some("upload-abc"));
        assert_eq!(data.key.as_deref(), Some("drive/2026/abc"));
        assert_eq!(data.storage_node.as_deref(), Some("node-1"));
        assert_eq!(data.slice_size, Some(16 * 1024 * 1024));
        assert_eq!(data.file_id, Some(42));
    }

    #[test]
    fn from_request_requires_core_fields() {
        let mut data = UploadRequestData {
            reuse: false,
            info: None,
            upload_id: Some("u".into()),
            bucket: Some("b".into()),
            key: Some("k".into()),
            storage_node: None,
            file_id: None,
            slice_size: None,
            upload_file_status: None,
        };
        assert!(UploadSession::from_request(&data, "md5", "f.bin", 1, 2).is_some());

        data.key = None;
        assert!(UploadSession::from_request(&data, "md5", "f.bin", 1, 2).is_none());
    }
}
