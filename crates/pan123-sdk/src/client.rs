use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use md5::Context;
use qrcode::QrCode;
use qrcode::render::unicode;
use rand::Rng;
use reqwest::blocking::{Client, Response};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, HeaderMap, HeaderValue, USER_AGENT};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use url::Url;
use uuid::Uuid;
use walkdir::WalkDir;

use crate::config;
use crate::error::{Pan123Error, Result};
use crate::models::{
    ApiEnvelope, CopyTaskCreateData, CopyTaskStatusData, DispatchItem, DomainData,
    DownloadInfoData, DownloadLink, DownloadResumeMeta, DownloadedFile, DuplicateMode, FileInfo,
    FileInfoListData, FileListData, PresignedUrlsData, QrGenerateData, QrResultData, SignInData,
    TrafficCheckData, UploadCompleteData, UploadRequestData, UserInfo, WxCodeData,
};
use crate::rate_limiter::{RateLimiter, RateLimiterConfig};
use crate::transfer::{
    DownloadOptions, ProgressCallback, RetryPolicy, TransferEvent, TransferFailure, TransferKind,
    UploadDirectoryReport, UploadFailureKind, UploadOptions,
};

const DEFAULT_BASE_URL: &str = "https://www.123pan.com/api";
const DEFAULT_UCENTER_URL: &str = "https://login.123pan.com";

#[derive(Debug, Clone)]
pub enum TokenCheckStatus {
    Missing,
    Valid,
    Invalid,
    Unreachable(String),
}

#[derive(Clone)]
pub struct Pan123Client {
    client: Client,
    base_url: String,
    ucenter_url: String,
    login_uuid: String,
    token: Option<String>,
    rate_limiter: Option<RateLimiter>,
}

impl Pan123Client {
    pub fn new(token: Option<String>) -> Result<Self> {
        Self::with_rate_limiter(token, None)
    }

    pub fn with_rate_limiter(
        token: Option<String>,
        rate_limiter_config: Option<RateLimiterConfig>,
    ) -> Result<Self> {
        let login_uuid = format!("{:x}", md5::compute(Uuid::new_v4().to_string()));

        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36 Edg/146.0.0.0"));
        headers.insert(
            "Accept",
            HeaderValue::from_static("application/json, text/plain, */*"),
        );
        headers.insert(
            "Accept-Language",
            HeaderValue::from_static("zh-CN,zh;q=0.9,en;q=0.8"),
        );
        headers.insert("App-Version", HeaderValue::from_static("3"));
        headers.insert("Origin", HeaderValue::from_static("https://www.123pan.com"));
        headers.insert(
            "Referer",
            HeaderValue::from_static("https://www.123pan.com/"),
        );
        headers.insert("platform", HeaderValue::from_static("web"));
        headers.insert(
            "LoginUuid",
            HeaderValue::from_str(&login_uuid).unwrap_or(HeaderValue::from_static("")),
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let client = Client::builder()
            .cookie_store(true)
            .default_headers(headers)
            .build()?;

        let rate_limiter =
            rate_limiter_config.map(|config| RateLimiter::new(config.api_requests_per_second));

        let mut instance = Self {
            client,
            base_url: DEFAULT_BASE_URL.to_string(),
            ucenter_url: DEFAULT_UCENTER_URL.to_string(),
            login_uuid,
            token: token.or_else(config::load_token),
            rate_limiter,
        };

        instance.init_domains()?;
        Ok(instance)
    }

    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    pub fn set_token(&mut self, token: impl Into<String>) -> Result<()> {
        let token = token.into();
        config::save_token(&token)?;
        self.token = Some(token);
        Ok(())
    }

    pub fn check_token_valid(&self) -> bool {
        matches!(self.check_token_status(), TokenCheckStatus::Valid)
    }

    pub fn check_token_status(&self) -> TokenCheckStatus {
        if self.token.is_none() {
            return TokenCheckStatus::Missing;
        }
        match self.get_file_list(0, 1, 1) {
            Ok(_) => TokenCheckStatus::Valid,
            Err(Pan123Error::Api { .. }) | Err(Pan123Error::AuthRequired) => {
                TokenCheckStatus::Invalid
            }
            Err(Pan123Error::Http { .. }) => {
                TokenCheckStatus::Unreachable("network error".to_string())
            }
            Err(err) => TokenCheckStatus::Unreachable(err.to_string()),
        }
    }

    pub fn login_by_qrcode(&mut self) -> Result<()> {
        let url = format!("{}/api/user/qr-code/generate", self.ucenter_url);
        let res: ApiEnvelope<QrGenerateData> =
            self.send_json(self.client.get(url), None::<Value>)?;
        let data = self.unwrap_data(res)?;
        let scan_url = format!(
            "https://www.123pan.com/wx-app-login.html?env=production&uniID={}&source=123pan&type=login",
            data.uni_id
        );

        self.print_qr_code(&scan_url)?;
        self.poll_qr_result(&data.uni_id)
    }

    pub fn get_user_info(&self) -> Result<UserInfo> {
        let url = format!("{}/b/api/restful/goapi/v1/user/report/info", self.domain());
        let res: ApiEnvelope<UserInfo> = self.send_json(
            self.client.get(url).query(&self.dynamic_params()),
            None::<Value>,
        )?;
        self.unwrap_data(res)
    }

    pub fn get_file_info(&self, file_ids: &[u64]) -> Result<Vec<FileInfo>> {
        let url = format!("{}/b/api/file/info", self.domain());
        let payload = json!({
            "fileIdList": file_ids.iter().map(|id| json!({"fileId": id})).collect::<Vec<_>>()
        });
        let res: ApiEnvelope<FileInfoListData> = self.send_json(
            self.client.post(url).query(&self.dynamic_params()),
            Some(payload),
        )?;
        Ok(self.unwrap_data(res)?.info_list)
    }

    pub fn get_file_list(&self, parent_id: u64, page: u32, limit: u32) -> Result<Vec<FileInfo>> {
        let url = format!("{}/b/api/file/list/new", self.domain());
        let mut params = HashMap::from([
            ("driveId".to_string(), "0".to_string()),
            ("limit".to_string(), limit.to_string()),
            ("next".to_string(), "0".to_string()),
            ("orderBy".to_string(), "update_time".to_string()),
            ("orderDirection".to_string(), "desc".to_string()),
            ("parentFileId".to_string(), parent_id.to_string()),
            ("trashed".to_string(), "false".to_string()),
            ("Page".to_string(), page.to_string()),
            ("operateType".to_string(), "1".to_string()),
        ]);
        params.extend(self.dynamic_params());
        let res: ApiEnvelope<FileListData> =
            self.send_json(self.client.get(url).query(&params), None::<Value>)?;
        Ok(self.unwrap_data(res)?.info_list)
    }

    pub fn rename_file(&self, file_id: u64, new_name: &str) -> Result<()> {
        let url = format!("{}/b/api/file/rename", self.domain());
        let payload = json!({
            "driveId": 0,
            "fileId": file_id,
            "fileName": new_name,
            "duplicate": 1,
            "RequestSource": Value::Null
        });
        let res: ApiEnvelope<Value> = self.send_json(
            self.client.post(url).query(&self.dynamic_params()),
            Some(payload),
        )?;
        self.ensure_ok(res)
    }

    pub fn move_files(&self, file_ids: &[u64], target_parent_id: u64) -> Result<()> {
        let url = format!("{}/b/api/file/mod_pid", self.domain());
        let payload = json!({
            "fileIdList": file_ids.iter().map(|id| json!({"FileId": id})).collect::<Vec<_>>(),
            "parentFileId": target_parent_id,
            "event": "fileMove",
            "operatePlace": "bottom",
            "RequestSource": Value::Null
        });
        let res: ApiEnvelope<Value> = self.send_json(
            self.client.post(url).query(&self.dynamic_params()),
            Some(payload),
        )?;
        self.ensure_ok(res)
    }

    pub fn delete_files(&self, file_ids: &[u64]) -> Result<()> {
        let info_list = self.get_file_info(file_ids)?;
        if info_list.is_empty() {
            return Err(Pan123Error::Operation(
                "failed to load file metadata before delete".into(),
            ));
        }

        let url = format!("{}/b/api/file/trash", self.domain());
        let payload = json!({
            "driveId": 0,
            "fileTrashInfoList": info_list,
            "operation": true,
            "event": "intoRecycle",
            "operatePlace": "bottom",
            "RequestSource": Value::Null,
            "safeBox": false
        });
        let res: ApiEnvelope<Value> = self.send_json(
            self.client.post(url).query(&self.dynamic_params()),
            Some(payload),
        )?;
        self.ensure_ok(res)
    }

    pub fn copy_files(&self, file_ids: &[u64], target_parent_id: u64) -> Result<()> {
        let full_infos = self.get_file_info(file_ids)?;
        if full_infos.is_empty() {
            return Err(Pan123Error::Operation(
                "failed to load file metadata before copy".into(),
            ));
        }

        let payload = json!({
            "fileList": full_infos.iter().map(|info| json!({
                "fileId": info.file_id,
                "size": info.size,
                "etag": info.etag,
                "type": info.file_type,
                "parentFileId": info.parent_file_id,
                "fileName": info.file_name,
                "driveId": 0
            })).collect::<Vec<_>>(),
            "targetFileId": target_parent_id
        });

        let url = format!("{}/b/api/restful/goapi/v1/file/copy/async", self.domain());
        let res: ApiEnvelope<CopyTaskCreateData> = self.send_json(
            self.client.post(url).query(&self.dynamic_params()),
            Some(payload),
        )?;
        let task_id = self
            .unwrap_data(res)?
            .task_id
            .ok_or_else(|| Pan123Error::Operation("copy task id missing from response".into()))?;

        let task_url = format!("{}/b/api/restful/goapi/v1/file/copy/task", self.domain());
        for _ in 0..30 {
            let mut params = HashMap::from([("taskId".to_string(), task_id.clone())]);
            params.extend(self.dynamic_params());
            let status_res: ApiEnvelope<CopyTaskStatusData> =
                self.send_json(self.client.get(&task_url).query(&params), None::<Value>)?;
            let status = self.unwrap_data(status_res)?.status.unwrap_or_default();
            if status == 2 {
                return Ok(());
            }
            thread::sleep(Duration::from_secs(1));
        }

        Err(Pan123Error::Operation("copy task timed out".into()))
    }

    pub fn create_folder(&self, folder_name: &str, parent_id: u64) -> Result<FileInfo> {
        let url = format!("{}/b/api/file/upload_request", self.domain());
        let payload = json!({
            "driveId": 0,
            "etag": "",
            "fileName": folder_name,
            "parentFileId": parent_id,
            "size": 0,
            "type": 1,
            "duplicate": 1,
            "NotReuse": true,
            "RequestSource": Value::Null
        });
        let res: ApiEnvelope<Value> = self.send_json(
            self.client.post(url).query(&self.dynamic_params()),
            Some(payload),
        )?;
        let data = self.unwrap_data(res)?;
        let info = data.get("Info").ok_or_else(|| {
            Pan123Error::Operation("folder info missing from create response".into())
        })?;
        parse_file_info_value(info).ok_or_else(|| {
            Pan123Error::Operation("failed to parse folder info from create response".into())
        })
    }

    pub fn get_download_link(&self, file_ids: &[u64]) -> Result<DownloadLink> {
        if file_ids.is_empty() {
            return Err(Pan123Error::Operation(
                "at least one file id is required".into(),
            ));
        }

        self.check_download_traffic(file_ids)?;

        let single_file = if file_ids.len() == 1 {
            self.get_file_info(file_ids)?
                .into_iter()
                .next()
                .filter(|it| !it.is_dir())
        } else {
            None
        };

        let (url, payload) = if let Some(info) = single_file.as_ref() {
            (
                format!("{}/b/api/v2/file/download_info", self.domain()),
                json!({
                    "driveId": 0,
                    "etag": info.etag.clone().unwrap_or_default(),
                    "fileId": info.file_id,
                    "s3keyFlag": info.s3_key_flag.clone().unwrap_or_default(),
                    "type": info.file_type,
                    "fileName": info.file_name,
                    "size": info.size,
                }),
            )
        } else {
            (
                format!("{}/b/api/v2/file/batch_download_info", self.domain()),
                json!({
                    "fileIdList": file_ids.iter().map(|id| json!({"fileId": id})).collect::<Vec<_>>()
                }),
            )
        };

        let res: ApiEnvelope<DownloadInfoData> = self.send_json(
            self.client.post(url).query(&self.dynamic_params()),
            Some(payload),
        )?;
        let data = self.unwrap_data(res)?;
        let dispatch = data
            .dispatch_list
            .first()
            .unwrap_or(&DispatchItem::default())
            .prefix
            .clone();
        if dispatch.is_empty() || data.download_path.is_empty() {
            return Err(Pan123Error::Operation(
                "download link missing from response".into(),
            ));
        }

        let final_url = format!("{}{}", dispatch, data.download_path);
        let filename = self.filename_from_download_url(&final_url, single_file.is_none())?;
        Ok(DownloadLink {
            url: final_url,
            filename,
        })
    }

    pub fn download_files(
        &self,
        file_ids: &[u64],
        save_dir: impl AsRef<Path>,
    ) -> Result<DownloadedFile> {
        self.download_files_with(file_ids, save_dir, DownloadOptions::default(), None)
    }

    pub fn download_files_with(
        &self,
        file_ids: &[u64],
        save_dir: impl AsRef<Path>,
        options: DownloadOptions,
        progress: Option<ProgressCallback>,
    ) -> Result<DownloadedFile> {
        let options = options.normalized();
        let link = self.retry_with_backoff(options.transfer.retry, "download-link", |_, _| {
            self.get_download_link(file_ids)
        })?;
        let save_path = save_dir.as_ref().join(&link.filename);
        let transfer_id = format!("download:{}", link.filename);
        let temp_path = save_path.with_extension("part");
        let meta_path = config::resume_meta_path_for(&save_path);

        let head_response = self.client.head(&link.url).send().ok();
        let total_hint = head_response.as_ref().and_then(|resp| {
            resp.headers()
                .get(reqwest::header::CONTENT_LENGTH)?
                .to_str()
                .ok()?
                .parse::<u64>()
                .ok()
        });
        let etag = head_response.as_ref().and_then(|resp| {
            resp.headers()
                .get(reqwest::header::ETAG)?
                .to_str()
                .ok()
                .map(|s| s.to_string())
        });
        let last_modified = head_response.as_ref().and_then(|resp| {
            resp.headers()
                .get(reqwest::header::LAST_MODIFIED)?
                .to_str()
                .ok()
                .map(|s| s.to_string())
        });

        let resume_meta = load_resume_meta(&meta_path);
        let mut should_restart = false;

        if let Some(meta) = &resume_meta {
            should_restart = !same_resume_target(meta, &link.url, &link.filename)
                || (meta.total_bytes.is_some()
                    && total_hint.is_some()
                    && meta.total_bytes != total_hint)
                || (etag.is_some() && meta.etag.is_some() && meta.etag != etag)
                || (last_modified.is_some()
                    && meta.last_modified.is_some()
                    && meta.last_modified != last_modified);

            if !should_restart && temp_path.exists() {
                let actual_size = fs::metadata(&temp_path).map(|m| m.len()).unwrap_or(0);
                if actual_size != meta.downloaded_bytes {
                    should_restart = true;
                }
            }
        }

        if should_restart {
            let _ = fs::remove_file(&temp_path);
            let _ = fs::remove_file(&meta_path);
        }

        if options.resume
            && temp_path.exists()
            && total_hint.is_some()
            && fs::metadata(&temp_path)
                .map(|meta| meta.len() == total_hint.unwrap_or(0))
                .unwrap_or(false)
        {
            fs::rename(&temp_path, &save_path)?;
            let _ = fs::remove_file(&meta_path);
            let file = DownloadedFile {
                file_path: save_path.clone(),
                size: total_hint.unwrap_or(0),
            };
            self.emit(
                &progress,
                TransferEvent::Started {
                    id: transfer_id.clone(),
                    kind: TransferKind::Download,
                    path: save_path.clone(),
                    total_bytes: total_hint,
                },
            );
            self.emit(
                &progress,
                TransferEvent::Finished {
                    id: transfer_id,
                    kind: TransferKind::Download,
                    path: file.file_path.clone(),
                    total_bytes: file.size,
                },
            );
            return Ok(file);
        }

        self.emit(
            &progress,
            TransferEvent::Started {
                id: transfer_id.clone(),
                kind: TransferKind::Download,
                path: save_path.clone(),
                total_bytes: total_hint,
            },
        );

        let result = self.retry_with_backoff_emit(
            options.transfer.retry,
            &transfer_id,
            TransferKind::Download,
            &progress,
            |_, _| {
                let existing = if options.resume && temp_path.exists() {
                    fs::metadata(&temp_path).map(|meta| meta.len()).unwrap_or(0)
                } else {
                    0
                };

                if let Some(total) = total_hint
                    && existing > total
                {
                    let _ = fs::remove_file(&temp_path);
                }
                let existing = if options.resume && temp_path.exists() {
                    fs::metadata(&temp_path).map(|meta| meta.len()).unwrap_or(0)
                } else {
                    0
                };

                let mut request = self.client.get(&link.url);
                if existing > 0 {
                    request = request.header(reqwest::header::RANGE, format!("bytes={existing}-"));
                }

                let mut response = request.send()?.error_for_status()?;
                let status = response.status();
                let supports_resume = status == reqwest::StatusCode::PARTIAL_CONTENT;
                let total_bytes = response
                    .headers()
                    .get(reqwest::header::CONTENT_RANGE)
                    .and_then(|value| value.to_str().ok())
                    .and_then(parse_total_from_content_range)
                    .or_else(|| {
                        response
                            .headers()
                            .get(reqwest::header::CONTENT_LENGTH)
                            .and_then(|value| value.to_str().ok())
                            .and_then(|value| value.parse::<u64>().ok())
                            .map(|len| if supports_resume { len + existing } else { len })
                    })
                    .or(total_hint);

                if existing > 0 && !supports_resume {
                    let _ = fs::remove_file(&temp_path);
                }

                let mut output = if existing > 0 && supports_resume {
                    fs::OpenOptions::new().append(true).open(&temp_path)?
                } else {
                    File::create(&temp_path)?
                };
                save_resume_meta(
                    &meta_path,
                    &DownloadResumeMeta {
                        url: link.url.clone(),
                        filename: link.filename.clone(),
                        total_bytes,
                        etag: etag.clone(),
                        last_modified: last_modified.clone(),
                        downloaded_bytes: existing,
                        created_at: chrono::Utc::now().timestamp(),
                    },
                )?;
                let mut buffer = vec![0u8; 256 * 1024];
                let mut downloaded = if existing > 0 && supports_resume {
                    existing
                } else {
                    0
                };

                loop {
                    let read = response.read(&mut buffer)?;
                    if read == 0 {
                        break;
                    }
                    output.write_all(&buffer[..read])?;
                    downloaded += read as u64;

                    if downloaded % (1024 * 1024) == 0 {
                        save_resume_meta(
                            &meta_path,
                            &DownloadResumeMeta {
                                url: link.url.clone(),
                                filename: link.filename.clone(),
                                total_bytes,
                                etag: etag.clone(),
                                last_modified: last_modified.clone(),
                                downloaded_bytes: downloaded,
                                created_at: chrono::Utc::now().timestamp(),
                            },
                        )?;
                    }

                    self.emit(
                        &progress,
                        TransferEvent::Progress {
                            id: transfer_id.clone(),
                            kind: TransferKind::Download,
                            bytes: downloaded,
                            total_bytes,
                        },
                    );
                }

                output.sync_all()?;
                fs::rename(&temp_path, &save_path)?;
                let _ = fs::remove_file(&meta_path);
                Ok(DownloadedFile {
                    file_path: save_path.clone(),
                    size: downloaded,
                })
            },
        );

        match result {
            Ok(file) => {
                self.emit(
                    &progress,
                    TransferEvent::Finished {
                        id: transfer_id,
                        kind: TransferKind::Download,
                        path: file.file_path.clone(),
                        total_bytes: file.size,
                    },
                );
                Ok(file)
            }
            Err(err) => {
                self.emit(
                    &progress,
                    TransferEvent::Failed {
                        id: transfer_id,
                        kind: TransferKind::Download,
                        path: save_path,
                        message: err.to_string(),
                    },
                );
                Err(err)
            }
        }
    }

    pub fn upload_file(
        &self,
        file_path: impl AsRef<Path>,
        parent_id: u64,
        duplicate: DuplicateMode,
    ) -> Result<FileInfo> {
        self.upload_file_with(
            file_path,
            parent_id,
            duplicate,
            UploadOptions::default(),
            None,
        )
    }

    pub fn upload_file_with(
        &self,
        file_path: impl AsRef<Path>,
        parent_id: u64,
        duplicate: DuplicateMode,
        options: UploadOptions,
        progress: Option<ProgressCallback>,
    ) -> Result<FileInfo> {
        let file_path = file_path.as_ref().to_path_buf();
        let transfer_id = format!("upload:{}", file_path.display());
        let total_bytes = fs::metadata(&file_path)?.len();
        self.emit(
            &progress,
            TransferEvent::Started {
                id: transfer_id.clone(),
                kind: TransferKind::Upload,
                path: file_path.clone(),
                total_bytes: Some(total_bytes),
            },
        );

        let result = self.upload_file_inner(
            &file_path,
            parent_id,
            duplicate,
            options.transfer.retry,
            progress.clone(),
            &transfer_id,
            options.transfer.parallelism,
        );
        match result {
            Ok(file) => {
                self.emit(
                    &progress,
                    TransferEvent::Finished {
                        id: transfer_id,
                        kind: TransferKind::Upload,
                        path: file_path,
                        total_bytes,
                    },
                );
                Ok(file)
            }
            Err(err) => {
                self.emit(
                    &progress,
                    TransferEvent::Failed {
                        id: transfer_id,
                        kind: TransferKind::Upload,
                        path: file_path,
                        message: err.to_string(),
                    },
                );
                Err(err)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn upload_file_inner(
        &self,
        file_path: &Path,
        parent_id: u64,
        duplicate: DuplicateMode,
        retry: RetryPolicy,
        progress: Option<ProgressCallback>,
        transfer_id: &str,
        parallelism: usize,
    ) -> Result<FileInfo> {
        let metadata = fs::metadata(file_path)?;
        if !metadata.is_file() {
            return Err(Pan123Error::InvalidPath(file_path.display().to_string()));
        }

        let file_name = file_path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| Pan123Error::InvalidPath(file_path.display().to_string()))?
            .to_string();
        let file_size = metadata.len();
        let md5_hash = self.calculate_file_md5(file_path)?;

        let request_url = format!("{}/b/api/file/upload_request", self.domain());
        let mut payload = json!({
            "driveId": 0,
            "etag": md5_hash,
            "fileName": file_name,
            "parentFileId": parent_id,
            "size": file_size,
            "type": 0
        });

        let mut response: ApiEnvelope<UploadRequestData> = self.retry_with_backoff_emit(
            retry,
            transfer_id,
            TransferKind::Upload,
            &progress,
            |_, _| {
                self.send_json(
                    self.client.post(&request_url).query(&self.dynamic_params()),
                    Some(payload.clone()),
                )
            },
        )?;
        if response.code == 5060 {
            if duplicate == DuplicateMode::Cancel {
                return Err(Pan123Error::Operation(
                    "upload canceled because target already exists".into(),
                ));
            }
            payload["duplicate"] = json!(duplicate.as_i64());
            response = self.retry_with_backoff_emit(
                retry,
                transfer_id,
                TransferKind::Upload,
                &progress,
                |_, _| {
                    self.send_json(
                        self.client.post(&request_url).query(&self.dynamic_params()),
                        Some(payload.clone()),
                    )
                },
            )?;
        }
        if response.code != 0 {
            return Err(Pan123Error::Api {
                code: response.code,
                message: response.message,
            });
        }

        let data = self.unwrap_data(response)?;
        if data.reuse {
            return data.info.ok_or_else(|| {
                Pan123Error::Operation("reuse upload returned without file info".into())
            });
        }

        let upload_id = data
            .upload_id
            .ok_or_else(|| Pan123Error::Operation("UploadId missing".into()))?;
        let bucket = data
            .bucket
            .ok_or_else(|| Pan123Error::Operation("Bucket missing".into()))?;
        let key = data
            .key
            .ok_or_else(|| Pan123Error::Operation("Key missing".into()))?;
        let storage_node = data
            .storage_node
            .ok_or_else(|| Pan123Error::Operation("StorageNode missing".into()))?;
        let temp_file_id = data
            .file_id
            .ok_or_else(|| Pan123Error::Operation("temporary FileId missing".into()))?;
        let slice_size = data.slice_size.unwrap_or(16 * 1024 * 1024);
        let part_count = std::cmp::max(1, file_size.div_ceil(slice_size));
        let multipart = part_count > 1;

        let mut file = File::open(file_path)?;
        let mut buffer = vec![0u8; slice_size as usize];
        let mut uploaded = 0u64;

        let mut chunks = Vec::new();
        for part_number in 1..=part_count {
            let read_len = file.read(&mut buffer)?;
            chunks.push((part_number, buffer[..read_len].to_vec()));
        }

        let chunk_parallelism = parallelism.min(part_count as usize).max(1);
        let chunks_arc = Arc::new(Mutex::new(chunks));
        let (tx, rx) = mpsc::channel();
        let mut handles = Vec::new();

        for _ in 0..chunk_parallelism {
            let chunks = Arc::clone(&chunks_arc);
            let tx = tx.clone();
            let client = self.clone();
            let bucket = bucket.clone();
            let key = key.clone();
            let upload_id = upload_id.clone();
            let storage_node = storage_node.clone();
            let progress = progress.clone();
            let transfer_id = transfer_id.to_string();

            let handle = thread::spawn(move || {
                loop {
                    let next = {
                        let mut guard = chunks.lock().expect("chunk queue lock poisoned");
                        guard.pop()
                    };
                    let Some((part_number, chunk)) = next else {
                        break;
                    };

                    let auth_url = if multipart {
                        format!(
                            "{}/b/api/file/s3_repare_upload_parts_batch",
                            client.domain()
                        )
                    } else {
                        format!("{}/b/api/file/s3_upload_object/auth", client.domain())
                    };

                    let auth_payload = json!({
                        "bucket": bucket,
                        "key": key,
                        "partNumberStart": part_number,
                        "partNumberEnd": part_number + 1,
                        "uploadId": upload_id,
                        "StorageNode": storage_node
                    });

                    let result = client.retry_with_backoff_emit(
                        retry,
                        &transfer_id,
                        TransferKind::Upload,
                        &progress,
                        |_, _| {
                            let auth_res: ApiEnvelope<PresignedUrlsData> = client.send_json(
                                client
                                    .client
                                    .post(&auth_url)
                                    .query(&client.dynamic_params()),
                                Some(auth_payload.clone()),
                            )?;
                            let mut pre_signed = client.unwrap_data(auth_res)?.presigned_urls;
                            let put_url =
                                pre_signed.remove(&part_number.to_string()).ok_or_else(|| {
                                    Pan123Error::Operation(format!(
                                        "missing pre-signed url for part {part_number}"
                                    ))
                                })?;

                            client
                                .client
                                .put(&put_url)
                                .header("Content-Length", chunk.len().to_string())
                                .body(chunk.clone())
                                .send()?
                                .error_for_status()?;
                            Ok(chunk.len() as u64)
                        },
                    );

                    if tx.send((part_number, result)).is_err() {
                        break;
                    }
                }
            });
            handles.push(handle);
        }
        drop(tx);

        for (part_number, result) in rx {
            match result {
                Ok(bytes) => {
                    uploaded += bytes;
                    self.emit(
                        &progress,
                        TransferEvent::Progress {
                            id: transfer_id.to_string(),
                            kind: TransferKind::Upload,
                            bytes: uploaded,
                            total_bytes: Some(file_size),
                        },
                    );
                }
                Err(err) => {
                    for handle in handles {
                        let _ = handle.join();
                    }
                    return Err(err.with_context(format!("upload part {part_number} failed")));
                }
            }
        }

        for handle in handles {
            let _ = handle.join();
        }

        let complete_url = format!("{}/b/api/file/upload_complete/v2", self.domain());
        let complete_payload = json!({
            "fileId": temp_file_id,
            "bucket": bucket,
            "fileSize": file_size,
            "key": key,
            "isMultipart": multipart,
            "uploadId": upload_id,
            "StorageNode": storage_node
        });
        let complete_res: ApiEnvelope<UploadCompleteData> = self.retry_with_backoff_emit(
            retry,
            transfer_id,
            TransferKind::Upload,
            &progress,
            |_, _| {
                self.send_json(
                    self.client
                        .post(&complete_url)
                        .query(&self.dynamic_params()),
                    Some(complete_payload.clone()),
                )
            },
        )?;
        let complete_data = self.unwrap_data(complete_res)?;
        if let Some(file_info) = complete_data.file_info {
            return Ok(file_info);
        }

        for _ in 0..30 {
            let maybe_file = self.get_file_info(&[temp_file_id])?.into_iter().next();
            if let Some(file_info) = maybe_file
                && file_info.status.unwrap_or_default() == 0
            {
                return Ok(file_info);
            }
            thread::sleep(Duration::from_secs(1));
        }

        Err(Pan123Error::Operation(
            "upload verification timed out".into(),
        ))
    }

    pub fn upload_directory(
        &self,
        local_dir: impl AsRef<Path>,
        parent_id: u64,
        duplicate: DuplicateMode,
    ) -> Result<Vec<FileInfo>> {
        let report = self.upload_directory_with(
            local_dir,
            parent_id,
            duplicate,
            UploadOptions::default(),
            None,
        )?;
        if report.failed.is_empty() {
            Ok(report.uploaded)
        } else {
            Err(Pan123Error::Operation(format!(
                "directory upload completed with {} failure(s)",
                report.failed.len()
            )))
        }
    }

    pub fn upload_directory_with(
        &self,
        local_dir: impl AsRef<Path>,
        parent_id: u64,
        duplicate: DuplicateMode,
        options: UploadOptions,
        progress: Option<ProgressCallback>,
    ) -> Result<UploadDirectoryReport> {
        let local_dir = local_dir.as_ref();
        if !local_dir.is_dir() {
            return Err(Pan123Error::InvalidPath(local_dir.display().to_string()));
        }

        let root_name = local_dir
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| Pan123Error::InvalidPath(local_dir.display().to_string()))?;
        let root_remote = self.create_folder(root_name, parent_id)?;
        let mut mapping = HashMap::from([(local_dir.canonicalize()?, root_remote.file_id)]);
        let mut upload_tasks = Vec::<(PathBuf, u64)>::new();

        for entry in WalkDir::new(local_dir)
            .min_depth(1)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            let path = entry.path().canonicalize()?;
            let parent = path
                .parent()
                .ok_or_else(|| Pan123Error::InvalidPath(path.display().to_string()))?
                .to_path_buf();
            let remote_parent = *mapping.get(&parent).ok_or_else(|| {
                Pan123Error::Operation(format!("missing remote parent for {}", path.display()))
            })?;

            if entry.file_type().is_dir() {
                let folder_name = entry.file_name().to_string_lossy().to_string();
                let remote = self.create_folder(&folder_name, remote_parent)?;
                mapping.insert(path, remote.file_id);
            } else if entry.file_type().is_file() {
                upload_tasks.push((path, remote_parent));
            }
        }

        if upload_tasks.is_empty() {
            return Ok(UploadDirectoryReport::default());
        }

        let parallelism = options.transfer.parallelism.max(1).min(upload_tasks.len());
        let queue = Arc::new(Mutex::new(upload_tasks));
        let (tx, rx) = mpsc::channel();
        let mut handles = Vec::new();

        for _ in 0..parallelism {
            let queue = Arc::clone(&queue);
            let tx = tx.clone();
            let client = self.clone();
            let progress = progress.clone();
            let handle = thread::spawn(move || {
                loop {
                    let next = {
                        let mut guard = queue.lock().expect("upload queue lock poisoned");
                        guard.pop()
                    };
                    let Some((path, remote_parent)) = next else {
                        break;
                    };
                    let result = client.upload_file_with(
                        &path,
                        remote_parent,
                        duplicate,
                        options,
                        progress.clone(),
                    );
                    if tx.send((path, result)).is_err() {
                        break;
                    }
                }
            });
            handles.push(handle);
        }
        drop(tx);

        let mut uploaded = Vec::new();
        let mut failed = Vec::new();
        for (path, result) in rx {
            match result {
                Ok(info) => uploaded.push(info),
                Err(err) => failed.push(TransferFailure {
                    path,
                    kind: classify_upload_failure(&err),
                    message: err.to_string(),
                }),
            }
        }

        for handle in handles {
            let _ = handle.join();
        }

        Ok(UploadDirectoryReport { uploaded, failed })
    }

    pub fn find_child_folder_by_name(
        &self,
        parent_id: u64,
        folder_name: &str,
    ) -> Result<Option<FileInfo>> {
        let items = self.get_file_list(parent_id, 1, 200)?;
        Ok(items
            .into_iter()
            .find(|item| item.is_dir() && item.file_name == folder_name))
    }

    pub fn get_path_chain(&self, file_id: u64) -> Result<Vec<FileInfo>> {
        if file_id == 0 {
            return Ok(Vec::new());
        }

        let mut chain = Vec::new();
        let mut current = file_id;
        loop {
            let item = self
                .get_file_info(&[current])?
                .into_iter()
                .next()
                .ok_or_else(|| Pan123Error::NotFound(format!("file id {current}")))?;
            current = item.parent_file_id;
            chain.push(item);
            if current == 0 {
                break;
            }
        }
        chain.reverse();
        Ok(chain)
    }

    fn init_domains(&mut self) -> Result<()> {
        let url = format!("{}/api/dydomain", DEFAULT_UCENTER_URL);
        let res: ApiEnvelope<DomainData> = match self.send_json(self.client.get(url), None::<Value>)
        {
            Ok(res) => res,
            Err(_) => return Ok(()),
        };
        if res.code != 0 {
            return Ok(());
        }
        if let Some(data) = res.data {
            if let Some(domain) = data.domains.first() {
                self.base_url = format!("https://{domain}/api");
            }
            if let Some(domain) = data.ucenter_domain {
                self.ucenter_url = format!("https://{domain}");
            }
        }
        Ok(())
    }

    fn print_qr_code(&self, content: &str) -> Result<()> {
        let code = QrCode::new(content.as_bytes())
            .map_err(|err| Pan123Error::Operation(err.to_string()))?;
        let image = code.render::<unicode::Dense1x2>().quiet_zone(false).build();
        println!("请使用 123pan App 或微信扫码登录：\n{image}\n{content}");
        Ok(())
    }

    fn poll_qr_result(&mut self, uni_id: &str) -> Result<()> {
        let url = format!("{}/api/user/qr-code/result", self.ucenter_url);
        let mut last_status = None;

        for _ in 0..120 {
            let res: ApiEnvelope<QrResultData> = self.send_json(
                self.client.get(&url).query(&[("uniID", uni_id)]),
                None::<Value>,
            )?;
            let data = res.data.clone();
            if res.code == 200
                && let Some(token) = data.as_ref().and_then(|item| item.token.clone())
            {
                self.set_token(token)?;
                return Ok(());
            }

            if res.code == 0 {
                let status = data.and_then(|item| item.login_status);
                if status != last_status {
                    match status {
                        Some(1) => println!("二维码已扫描，请在手机上确认登录..."),
                        Some(4) => {
                            let wx_code_url =
                                format!("{}/api/user/qr-code/wx_code", self.ucenter_url);
                            let wx_res: ApiEnvelope<WxCodeData> = self.send_json(
                                self.client.post(wx_code_url),
                                Some(json!({"uniID": uni_id})),
                            )?;
                            if let Some(wx_code) = self.unwrap_data(wx_res)?.wx_code {
                                let sign_in_url = format!("{}/api/user/sign_in", self.ucenter_url);
                                let sign_res: ApiEnvelope<SignInData> = self.send_json(
                                    self.client.post(sign_in_url),
                                    Some(json!({"from": "web", "wechat_code": wx_code, "type": 4})),
                                )?;
                                if (sign_res.code == 0 || sign_res.code == 200)
                                    && let Some(token) = sign_res.data.and_then(|data| data.token)
                                {
                                    self.set_token(token)?;
                                    return Ok(());
                                }
                            }
                            return Err(Pan123Error::Operation(
                                "wechat qr login did not return a token".into(),
                            ));
                        }
                        _ => {}
                    }
                    last_status = status;
                }
            }

            if matches!(res.code, 401 | 403 | 404) {
                return Err(Pan123Error::AuthRequired);
            }

            thread::sleep(Duration::from_millis(1500));
        }

        Err(Pan123Error::Operation("二维码登录超时".into()))
    }

    fn check_download_traffic(&self, file_ids: &[u64]) -> Result<()> {
        let url = format!("{}/b/api/file/download/traffic/check", self.domain());
        let res: ApiEnvelope<TrafficCheckData> = self.send_json(
            self.client.post(url).query(&self.dynamic_params()),
            Some(json!({"fids": file_ids})),
        )?;
        let data = self.unwrap_data(res)?;
        if data.is_traffic_exceeded {
            return Err(Pan123Error::Operation("download traffic exceeded".into()));
        }
        Ok(())
    }

    fn calculate_file_md5(&self, file_path: &Path) -> Result<String> {
        let mut context = Context::new();
        let mut file = File::open(file_path)?;
        let mut buffer = [0u8; 4 * 1024 * 1024];

        loop {
            let bytes = file.read(&mut buffer)?;
            if bytes == 0 {
                break;
            }
            context.consume(&buffer[..bytes]);
        }

        Ok(format!("{:x}", context.finalize()))
    }

    fn filename_from_download_url(&self, url: &str, append_zip: bool) -> Result<String> {
        let parsed = Url::parse(url).map_err(|err| Pan123Error::Operation(err.to_string()))?;
        let name = parsed
            .query_pairs()
            .find_map(|(key, value)| (key == "filename").then(|| value.to_string()))
            .unwrap_or_else(|| "downloaded_file".to_string());
        Ok(if append_zip {
            format!("{name}.zip")
        } else {
            name
        })
    }

    fn dynamic_params(&self) -> HashMap<String, String> {
        let mut rng = rand::rng();
        let key = rng.random_range(0..i32::MAX);
        let value = format!(
            "{}-{}-{}",
            chrono::Utc::now().timestamp(),
            rng.random_range(0..10_000_000u64),
            rng.random_range(0..10_000_000_000u64)
        );
        HashMap::from([(key.to_string(), value)])
    }

    fn domain(&self) -> &str {
        self.base_url
            .strip_suffix("/api")
            .unwrap_or("https://www.123pan.com")
    }

    fn with_auth(
        &self,
        builder: reqwest::blocking::RequestBuilder,
    ) -> reqwest::blocking::RequestBuilder {
        let builder = builder.header("LoginUuid", &self.login_uuid);
        if let Some(token) = self.token() {
            builder
                .header(AUTHORIZATION, format!("Bearer {token}"))
                .header(COOKIE, format!("sso-token={token}"))
        } else {
            builder
        }
    }

    fn send_json<T, B>(
        &self,
        builder: reqwest::blocking::RequestBuilder,
        body: Option<B>,
    ) -> Result<T>
    where
        T: DeserializeOwned,
        B: Serialize,
    {
        if let Some(limiter) = &self.rate_limiter {
            limiter.acquire();
        }

        let request = self.with_auth(builder);
        let response = if let Some(body) = body {
            request.json(&body).send()?
        } else {
            request.send()?
        };
        Self::decode_json(response)
    }

    fn decode_json<T: DeserializeOwned>(response: Response) -> Result<T> {
        let response = response.error_for_status()?;
        Ok(response.json()?)
    }

    fn unwrap_data<T>(&self, envelope: ApiEnvelope<T>) -> Result<T> {
        if envelope.code != 0 {
            return Err(Pan123Error::Api {
                code: envelope.code,
                message: envelope.message,
            });
        }
        envelope
            .data
            .ok_or_else(|| Pan123Error::Operation("missing data field in api response".into()))
    }

    fn ensure_ok<T>(&self, envelope: ApiEnvelope<T>) -> Result<()> {
        if envelope.code == 0 {
            Ok(())
        } else {
            Err(Pan123Error::Api {
                code: envelope.code,
                message: envelope.message,
            })
        }
    }

    fn emit(&self, progress: &Option<ProgressCallback>, event: TransferEvent) {
        if let Some(callback) = progress {
            callback(event);
        }
    }

    fn retry_with_backoff<T, F>(&self, policy: RetryPolicy, _id: &str, mut op: F) -> Result<T>
    where
        F: FnMut(usize, bool) -> Result<T>,
    {
        let max_attempts = policy.max_attempts.max(1);
        let mut last_err = None;
        for attempt in 1..=max_attempts {
            match op(attempt, attempt > 1) {
                Ok(value) => return Ok(value),
                Err(err) => {
                    if attempt == max_attempts || !Self::is_retryable_error(&err) {
                        return Err(err);
                    }
                    last_err = Some(err);
                    let backoff = policy.calculate_delay(attempt);
                    thread::sleep(Duration::from_millis(backoff));
                }
            }
        }
        Err(last_err.unwrap_or_else(|| Pan123Error::Operation("retry failed".into())))
    }

    fn retry_with_backoff_emit<T, F>(
        &self,
        policy: RetryPolicy,
        id: &str,
        kind: TransferKind,
        progress: &Option<ProgressCallback>,
        mut op: F,
    ) -> Result<T>
    where
        F: FnMut(usize, bool) -> Result<T>,
    {
        let max_attempts = policy.max_attempts.max(1);
        let mut last_err = None;
        for attempt in 1..=max_attempts {
            match op(attempt, attempt > 1) {
                Ok(value) => return Ok(value),
                Err(err) => {
                    if attempt == max_attempts || !Self::is_retryable_error(&err) {
                        return Err(err);
                    }
                    self.emit(
                        progress,
                        TransferEvent::Retrying {
                            id: id.to_string(),
                            kind,
                            attempt,
                            message: err.to_string(),
                        },
                    );
                    last_err = Some(err);
                    let backoff = policy.calculate_delay(attempt);
                    thread::sleep(Duration::from_millis(backoff));
                }
            }
        }
        Err(last_err.unwrap_or_else(|| Pan123Error::Operation("retry failed".into())))
    }

    fn is_retryable_error(err: &Pan123Error) -> bool {
        err.is_retryable()
    }
}

fn parse_total_from_content_range(header: &str) -> Option<u64> {
    let (_, total) = header.split_once('/')?;
    total.parse::<u64>().ok()
}

fn load_resume_meta(path: &Path) -> Option<DownloadResumeMeta> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_resume_meta(path: &Path, meta: &DownloadResumeMeta) -> Result<()> {
    fs::write(path, serde_json::to_string_pretty(meta)?)?;
    Ok(())
}

fn same_resume_target(meta: &DownloadResumeMeta, url: &str, filename: &str) -> bool {
    meta.url == url && meta.filename == filename
}

fn parse_file_info_value(value: &Value) -> Option<FileInfo> {
    Some(FileInfo {
        file_id: value_u64(value, &["FileId", "fileId"])?,
        parent_file_id: value_u64(value, &["ParentFileId", "parentFileId"]).unwrap_or(0),
        file_name: value_string(value, &["FileName", "fileName"]).unwrap_or_default(),
        file_type: value_u64(value, &["Type", "type"]).unwrap_or(0) as u8,
        size: value_u64(value, &["Size", "size"]).unwrap_or(0),
        etag: value_string(value, &["Etag", "etag"]),
        s3_key_flag: value_string(value, &["S3KeyFlag", "s3KeyFlag"]),
        status: value_i64(value, &["Status", "status"]),
        extra: value.clone(),
    })
}

fn value_u64(value: &Value, keys: &[&str]) -> Option<u64> {
    let raw = value_get(value, keys)?;
    match raw {
        Value::Number(number) => number.as_u64(),
        Value::String(text) if !text.is_empty() => text.parse::<u64>().ok(),
        _ => None,
    }
}

fn value_i64(value: &Value, keys: &[&str]) -> Option<i64> {
    let raw = value_get(value, keys)?;
    match raw {
        Value::Number(number) => number.as_i64(),
        Value::String(text) if !text.is_empty() => text.parse::<i64>().ok(),
        _ => None,
    }
}

fn value_string(value: &Value, keys: &[&str]) -> Option<String> {
    let raw = value_get(value, keys)?;
    match raw {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(boolean) => Some(boolean.to_string()),
        _ => None,
    }
}

fn value_get<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    let object = value.as_object()?;
    keys.iter().find_map(|key| object.get(*key))
}

fn classify_upload_failure(err: &Pan123Error) -> UploadFailureKind {
    match err {
        Pan123Error::Io { .. } => UploadFailureKind::LocalIo,
        Pan123Error::Http { .. } => UploadFailureKind::Network,
        Pan123Error::Api { code, .. } if *code == 5060 => UploadFailureKind::Conflict,
        Pan123Error::Api { .. } | Pan123Error::Json { .. } => UploadFailureKind::RemoteApi,
        Pan123Error::AuthRequired => UploadFailureKind::Auth,
        Pan123Error::InvalidPath(_) | Pan123Error::NotFound(_) => UploadFailureKind::Validation,
        Pan123Error::Operation(message) => {
            let text = message.to_ascii_lowercase();
            if text.contains("exist") || text.contains("duplicate") || text.contains("conflict") {
                UploadFailureKind::Conflict
            } else if text.contains("auth") || text.contains("login") || text.contains("token") {
                UploadFailureKind::Auth
            } else if text.contains("path") || text.contains("not found") {
                UploadFailureKind::Validation
            } else if text.contains("timeout")
                || text.contains("network")
                || text.contains("connect")
            {
                UploadFailureKind::Network
            } else {
                UploadFailureKind::Unknown
            }
        }
        _ => UploadFailureKind::Unknown,
    }
}
