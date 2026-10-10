use std::sync::atomic::Ordering;

use pan123_sdk::{FileInfo, Pan123Client, Pan123Error};
use serde_json::{Value, json};

use crate::mcp::protocol::{CallContext, ToolCallOutcome, ToolHandler, ToolSpec};

pub struct Pan123Tools {
    pub client: Pan123Client,
}

pub fn id_argument(value: &Value, key: &str) -> Result<u64, String> {
    match value {
        Value::Number(number) => number
            .as_u64()
            .ok_or_else(|| format!("参数 {key} 必须是正整数，收到 {number}")),
        Value::String(text) => text
            .trim()
            .parse::<u64>()
            .map_err(|_| format!("参数 {key} 不是合法的文件 ID：{text:?}")),
        other => Err(format!(
            "参数 {key} 必须是文件 ID（数字或字符串），收到 {other}"
        )),
    }
}

pub fn ids_argument(value: &Value, key: &str) -> Result<Vec<u64>, String> {
    let array = value
        .as_array()
        .ok_or_else(|| format!("参数 {key} 必须是文件 ID 数组"))?;
    if array.is_empty() {
        return Err(format!("参数 {key} 至少需要一个文件 ID"));
    }
    array
        .iter()
        .map(|item| id_argument(item, key))
        .collect::<Result<Vec<_>, _>>()
}

/// 把绝对路径拆成目录段。`/` 或空串返回空向量。拒绝相对路径与 `.` / `..` 段。
pub fn parse_path_segments(path: &str) -> Result<Vec<String>, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "/" {
        return Ok(Vec::new());
    }
    if !trimmed.starts_with('/') {
        return Err(format!("路径必须是绝对路径（以 / 开头），收到 {path:?}"));
    }
    let mut segments = Vec::new();
    for segment in trimmed.split('/') {
        if segment.is_empty() {
            continue;
        }
        if segment == "." || segment == ".." {
            return Err(format!("路径中不允许出现 . 或 .. 段：{path:?}"));
        }
        segments.push(segment.to_string());
    }
    Ok(segments)
}

pub fn format_file_list(items: &[FileInfo]) -> String {
    if items.is_empty() {
        return "该目录为空".to_string();
    }
    items
        .iter()
        .map(|item| {
            if item.is_dir() {
                format!("[目录] {} (id={})", item.file_name, item.file_id)
            } else {
                format!(
                    "[文件] {} (id={}, {} 字节)",
                    item.file_name, item.file_id, item.size
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 把 Pan123Error 翻译成给模型看的、可操作的说明。
pub fn explain_error(err: &Pan123Error) -> String {
    match err {
        Pan123Error::AuthRequired | Pan123Error::InvalidToken => {
            "登录状态无效。请让用户在终端执行 `pan123 login` 扫码登录后重试。".to_string()
        }
        Pan123Error::NotFound(what) => format!("未找到：{what}。请确认 file_id 是否正确。"),
        Pan123Error::Cancelled => "操作已被取消。".to_string(),
        Pan123Error::InvalidPath(path) => format!("路径不合法：{path}。"),
        other => format!("操作失败：{other}"),
    }
}

/// 把闭包里的 SDK 调用收敛成统一的 outcome，顺带做取消检查。
fn run_read_only<F>(ctx: &CallContext, operation: F) -> ToolCallOutcome
where
    F: FnOnce() -> Result<String, Pan123Error>,
{
    if ctx.cancel.load(Ordering::SeqCst) {
        return ToolCallOutcome {
            text: "操作已被取消。".into(),
            is_error: true,
        };
    }
    match operation() {
        Ok(text) => ToolCallOutcome {
            text,
            is_error: false,
        },
        Err(err) => ToolCallOutcome {
            text: explain_error(&err),
            is_error: true,
        },
    }
}

fn spec(
    name: &'static str,
    description: &'static str,
    properties: Value,
    required: &[&str],
) -> ToolSpec {
    ToolSpec {
        name,
        description,
        input_schema: json!({
            "type": "object",
            "properties": properties,
            "required": required
        }),
    }
}

fn id_property(description: &str) -> Value {
    json!({
        "type": ["integer", "string"],
        "description": format!("{description}（数字或字符串形式均可）")
    })
}

pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        spec(
            "list_files",
            "列出指定目录下的文件与子目录。parent_id 省略时使用根目录 0。",
            json!({
                "parent_id": id_property("目录的 file_id，根目录为 0"),
                "limit": { "type": "integer", "description": "返回条数上限，默认 100" }
            }),
            &[],
        ),
        spec(
            "dir_tree",
            "递归列出目录树。必须显式给出 depth，没有隐含的当前目录。",
            json!({
                "parent_id": id_property("起始目录的 file_id，根目录为 0"),
                "depth": { "type": "integer", "description": "递归深度，必须显式指定" }
            }),
            &["depth"],
        ),
        spec(
            "find_files",
            "在指定目录下按名称搜索文件与目录。",
            json!({
                "query": { "type": "string", "description": "搜索关键词" },
                "parent_id": id_property("搜索起点目录，根目录为 0"),
                "exact": { "type": "boolean", "description": "是否精确匹配名称，默认 false" },
                "dir_only": { "type": "boolean", "description": "只返回目录" },
                "file_only": { "type": "boolean", "description": "只返回文件" }
            }),
            &["query"],
        ),
        spec(
            "get_file_info",
            "按 file_id 批量查询文件详情。",
            json!({ "file_ids": {
                "type": "array",
                "description": "文件 ID 数组，元素可为数字或字符串",
                "items": { "type": ["integer", "string"] }
            }}),
            &["file_ids"],
        ),
        spec(
            "resolve_path",
            "把形如 /备份/2026 的路径解析为 file_id。无状态，每次操作前都需要解析。",
            json!({
                "path": { "type": "string", "description": "以 / 开头的绝对路径，/ 表示根目录" }
            }),
            &["path"],
        ),
        spec(
            "get_path",
            "由 file_id 反查完整路径，用于向用户复述文件位置。",
            json!({ "file_id": id_property("文件或目录的 ID") }),
            &["file_id"],
        ),
        spec("user_info", "查看当前登录账号的信息。", json!({}), &[]),
        spec(
            "check_token",
            "检查登录状态是否有效。返回 Missing/Invalid 时需要用户去终端执行 pan123 login。",
            json!({}),
            &[],
        ),
        spec(
            "create_folder",
            "在指定目录下新建文件夹。",
            json!({
                "folder_name": { "type": "string", "description": "新文件夹名称" },
                "parent_id": id_property("父目录的 file_id，根目录为 0")
            }),
            &["folder_name"],
        ),
        spec(
            "rename_file",
            "重命名文件或目录。这是修改操作，执行前必须先向用户确认。",
            json!({
                "file_id": id_property("要重命名的对象 ID"),
                "new_name": { "type": "string", "description": "新名称" }
            }),
            &["file_id", "new_name"],
        ),
        spec(
            "move_files",
            "批量移动文件或目录。这是破坏性操作，执行前必须先向用户确认。",
            json!({
                "file_ids": {
                    "type": "array",
                    "description": "要移动的对象 ID 数组",
                    "items": { "type": ["integer", "string"] }
                },
                "target_parent_id": id_property("目标目录的 file_id，根目录为 0")
            }),
            &["file_ids", "target_parent_id"],
        ),
        spec(
            "copy_files",
            "批量复制文件或目录到目标目录。",
            json!({
                "file_ids": {
                    "type": "array",
                    "description": "要复制的对象 ID 数组",
                    "items": { "type": ["integer", "string"] }
                },
                "target_parent_id": id_property("目标目录的 file_id，根目录为 0")
            }),
            &["file_ids", "target_parent_id"],
        ),
        spec(
            "delete_files",
            "批量删除文件或目录。这是不可逆操作，执行前必须先列出将被删除的对象并取得用户确认。",
            json!({
                "file_ids": {
                    "type": "array",
                    "description": "要删除的对象 ID 数组",
                    "items": { "type": ["integer", "string"] }
                }
            }),
            &["file_ids"],
        ),
        spec(
            "get_download_url",
            "获取文件的下载直链，不落盘。",
            json!({ "file_ids": {
                "type": "array",
                "description": "文件 ID 数组",
                "items": { "type": ["integer", "string"] }
            }}),
            &["file_ids"],
        ),
        spec(
            "download_files",
            "把云端文件下载到本地目录。耗时操作，会持续上报进度。dest_dir 必须是绝对路径。",
            json!({
                "file_ids": {
                    "type": "array",
                    "description": "文件 ID 数组",
                    "items": { "type": ["integer", "string"] }
                },
                "dest_dir": {
                    "type": "string",
                    "description": "本地目标目录，必须是绝对路径（相对于 server 进程工作目录解析）"
                },
                "retries": { "type": "integer", "description": "重试次数，默认 3" }
            }),
            &["file_ids", "dest_dir"],
        ),
        spec(
            "upload_file",
            "上传单个本地文件到云端目录。耗时操作，会持续上报进度。local_path 必须是绝对路径。",
            json!({
                "local_path": {
                    "type": "string",
                    "description": "本地文件路径，必须是绝对路径（相对于 server 进程工作目录解析）"
                },
                "parent_id": id_property("目标目录的 file_id，根目录为 0"),
                "duplicate": {
                    "type": "string",
                    "enum": ["keep_both", "overwrite", "cancel"],
                    "description": "遇到同名文件时的策略，默认 keep_both"
                }
            }),
            &["local_path"],
        ),
        spec(
            "upload_directory",
            "递归上传本地目录到云端。耗时操作，会持续上报进度。local_path 必须是绝对路径。",
            json!({
                "local_path": {
                    "type": "string",
                    "description": "本地目录路径，必须是绝对路径（相对于 server 进程工作目录解析）"
                },
                "parent_id": id_property("目标目录的 file_id，根目录为 0"),
                "duplicate": {
                    "type": "string",
                    "enum": ["keep_both", "overwrite", "cancel"],
                    "description": "遇到同名文件时的策略，默认 keep_both"
                },
                "retries": { "type": "integer", "description": "重试次数，默认 3" }
            }),
            &["local_path"],
        ),
        spec(
            "list_upload_sessions",
            "列出本地记录的未完成上传会话（断点续传依据）。",
            json!({ "clear": { "type": "boolean", "description": "是否清空全部会话记录" } }),
            &[],
        ),
    ]
}

impl ToolHandler for Pan123Tools {
    fn list_tools(&self) -> Vec<ToolSpec> {
        tool_specs()
    }

    fn call(&self, name: &str, args: &Value, ctx: &CallContext) -> ToolCallOutcome {
        match name {
            "list_files" => run_read_only(ctx, || {
                let parent_id = match args.get("parent_id") {
                    Some(value) => {
                        id_argument(value, "parent_id").map_err(Pan123Error::Operation)?
                    }
                    None => 0,
                };
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(100) as u32;
                let items = self.client.get_file_list(parent_id, 1, limit)?;
                Ok(format!(
                    "目录 {} 下共 {} 项：\n{}",
                    parent_id,
                    items.len(),
                    format_file_list(&items)
                ))
            }),
            "get_file_info" => run_read_only(ctx, || {
                let raw = args
                    .get("file_ids")
                    .ok_or_else(|| Pan123Error::Operation("缺少参数 file_ids".into()))?;
                let ids = ids_argument(raw, "file_ids").map_err(Pan123Error::Operation)?;
                let items = self.client.get_file_info(&ids)?;
                Ok(format_file_list(&items))
            }),
            "get_path" => run_read_only(ctx, || {
                let raw = args
                    .get("file_id")
                    .ok_or_else(|| Pan123Error::Operation("缺少参数 file_id".into()))?;
                let id = id_argument(raw, "file_id").map_err(Pan123Error::Operation)?;
                let chain = self.client.get_path_chain(id)?;
                let path = chain
                    .iter()
                    .map(|item| item.file_name.as_str())
                    .collect::<Vec<_>>()
                    .join("/");
                Ok(format!("/{} (id={})", path, id))
            }),
            "user_info" => run_read_only(ctx, || {
                Ok(serde_json::to_string_pretty(&self.client.get_user_info()?)?)
            }),
            "check_token" => {
                use pan123_sdk::TokenCheckStatus;
                let text = match self.client.check_token_status() {
                    TokenCheckStatus::Valid => "登录状态有效。".to_string(),
                    TokenCheckStatus::Missing => {
                        "尚未登录。请让用户在终端执行 `pan123 login` 扫码登录。".to_string()
                    }
                    TokenCheckStatus::Invalid => {
                        "登录已失效。请让用户在终端执行 `pan123 login` 重新扫码。".to_string()
                    }
                    TokenCheckStatus::Unreachable(message) => {
                        format!("无法验证登录状态（网络问题）：{message}")
                    }
                };
                ToolCallOutcome {
                    text,
                    is_error: false,
                }
            }
            "resolve_path" => run_read_only(ctx, || {
                let raw = args
                    .get("path")
                    .ok_or_else(|| Pan123Error::Operation("缺少参数 path".into()))?;
                let path = raw
                    .as_str()
                    .ok_or_else(|| Pan123Error::Operation("参数 path 必须是字符串".into()))?;
                let segments = parse_path_segments(path).map_err(Pan123Error::Operation)?;
                let mut current = 0u64;
                for segment in &segments {
                    let folder = self.client.find_child_folder_by_name(current, segment)?;
                    match folder {
                        Some(item) => current = item.file_id,
                        None => {
                            return Err(Pan123Error::NotFound(format!(
                                "路径 {path} 中的目录 {segment}"
                            )));
                        }
                    }
                }
                Ok(format!("{} -> file_id={}", path, current))
            }),
            other => ToolCallOutcome {
                text: format!("未知工具：{other}"),
                is_error: true,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn declares_eighteen_tools() {
        assert_eq!(tool_specs().len(), 18);
    }

    #[test]
    fn tool_names_are_unique_and_snake_case() {
        let mut seen = HashSet::new();
        for spec in tool_specs() {
            assert!(seen.insert(spec.name), "重复的工具名：{}", spec.name);
            assert!(
                spec.name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_'),
                "工具名必须是 snake_case：{}",
                spec.name
            );
        }
    }

    #[test]
    fn every_schema_is_an_object_with_properties() {
        for spec in tool_specs() {
            assert_eq!(
                spec.input_schema["type"], "object",
                "{} 的 inputSchema.type 必须是 object",
                spec.name
            );
            assert!(
                spec.input_schema["properties"].is_object(),
                "{} 缺少 properties",
                spec.name
            );
            assert!(!spec.description.is_empty(), "{} 缺少描述", spec.name);
        }
    }

    #[test]
    fn required_fields_exist_in_properties() {
        for spec in tool_specs() {
            let properties = spec.input_schema["properties"]
                .as_object()
                .expect("properties object");
            let Some(required) = spec.input_schema.get("required").and_then(Value::as_array) else {
                continue;
            };
            for field in required {
                let name = field.as_str().expect("required entries are strings");
                assert!(
                    properties.contains_key(name),
                    "{} 的 required 字段 {name} 不在 properties 中",
                    spec.name
                );
            }
        }
    }

    #[test]
    fn id_fields_accept_number_or_string() {
        assert_eq!(id_argument(&json!(123), "parent_id").unwrap(), 123);
        assert_eq!(id_argument(&json!("123"), "parent_id").unwrap(), 123);
    }

    #[test]
    fn id_argument_reports_the_offending_key() {
        let err = id_argument(&json!("not-a-number"), "parent_id").unwrap_err();
        assert!(err.contains("parent_id"), "错误信息应指明参数名：{err}");
    }

    #[test]
    fn id_argument_rejects_missing_value() {
        let err = id_argument(&json!(null), "file_id").unwrap_err();
        assert!(err.contains("file_id"));
    }

    #[test]
    fn id_argument_rejects_value_beyond_u64() {
        let err = id_argument(&json!("99999999999999999999999"), "file_id").unwrap_err();
        assert!(err.contains("file_id"));
    }

    #[test]
    fn ids_argument_accepts_arrays_of_mixed_forms() {
        let ids = ids_argument(&json!([1, "2", 3]), "file_ids").unwrap();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[test]
    fn ids_argument_rejects_empty_array() {
        let err = ids_argument(&json!([]), "file_ids").unwrap_err();
        assert!(err.contains("file_ids"));
    }

    #[test]
    fn ids_argument_rejects_non_array() {
        let err = ids_argument(&json!(5), "file_ids").unwrap_err();
        assert!(err.contains("file_ids"));
    }

    use pan123_sdk::FileInfo;
    use serde_json::json as j;

    /// 测试用的客户端。`Pan123Client::new` 会尽力做一次域名发现与服务器时间刷新，
    /// 两者离线时都软失败；用 OnceLock 让每个测试二进制至多尝试一次。
    fn test_client() -> Pan123Client {
        static CLIENT: std::sync::OnceLock<Pan123Client> = std::sync::OnceLock::new();
        CLIENT
            .get_or_init(|| {
                Pan123Client::new(Some("dummy-token".into())).expect("client builds offline")
            })
            .clone()
    }

    fn file(id: u64, parent: u64, name: &str, file_type: u8, size: u64) -> FileInfo {
        serde_json::from_value(j!({
            "FileId": id,
            "ParentFileId": parent,
            "FileName": name,
            "Type": file_type,
            "Size": size
        }))
        .expect("fixture should deserialize")
    }

    #[test]
    fn parse_path_segments_splits_absolute_path() {
        assert_eq!(
            parse_path_segments("/备份/2026").unwrap(),
            vec!["备份", "2026"]
        );
    }

    #[test]
    fn parse_path_segments_root_is_empty() {
        assert_eq!(parse_path_segments("/").unwrap(), Vec::<String>::new());
        assert_eq!(parse_path_segments("").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn parse_path_segments_rejects_relative_path() {
        let err = parse_path_segments("备份/2026").unwrap_err();
        assert!(err.contains("绝对路径"), "错误信息：{err}");
    }

    #[test]
    fn parse_path_segments_rejects_dot_segments() {
        assert!(parse_path_segments("/a/../b").is_err());
        assert!(parse_path_segments("/a/.").is_err());
    }

    #[test]
    fn format_file_list_marks_directories_and_sizes() {
        let items = vec![file(1, 0, "照片", 1, 0), file(2, 0, "报告.pdf", 0, 2048)];
        let text = format_file_list(&items);
        assert!(text.contains("照片"), "缺少目录名：{text}");
        assert!(text.contains("[目录]"), "缺少目录标记：{text}");
        assert!(text.contains("报告.pdf"), "缺少文件名：{text}");
        assert!(text.contains("2048"), "缺少大小：{text}");
    }

    #[test]
    fn format_file_list_reports_empty_directory() {
        assert!(format_file_list(&[]).contains("空"));
    }

    #[test]
    fn unknown_tool_returns_is_error() {
        let client = test_client();
        let tools = Pan123Tools { client };
        let ctx = crate::mcp::protocol::CallContext {
            progress: crate::mcp::protocol::ProgressSink::discarding(),
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        let outcome = tools.call("no_such_tool", &j!({}), &ctx);
        assert!(outcome.is_error);
        assert!(outcome.text.contains("no_such_tool"));
    }

    #[test]
    fn resolve_path_on_root_needs_no_api_call() {
        let client = test_client();
        let tools = Pan123Tools { client };
        let ctx = crate::mcp::protocol::CallContext {
            progress: crate::mcp::protocol::ProgressSink::discarding(),
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        let outcome = tools.call("resolve_path", &j!({ "path": "/" }), &ctx);
        assert!(
            !outcome.is_error,
            "根目录解析不应依赖网络：{}",
            outcome.text
        );
        assert!(outcome.text.contains('0'));
    }

    #[test]
    fn invalid_id_argument_returns_is_error_naming_the_key() {
        let client = test_client();
        let tools = Pan123Tools { client };
        let ctx = crate::mcp::protocol::CallContext {
            progress: crate::mcp::protocol::ProgressSink::discarding(),
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        let outcome = tools.call("get_file_info", &j!({ "file_ids": ["abc"] }), &ctx);
        assert!(outcome.is_error);
        assert!(outcome.text.contains("file_ids"), "实际：{}", outcome.text);
    }
}
