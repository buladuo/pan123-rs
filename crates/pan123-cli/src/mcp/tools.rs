use pan123_sdk::Pan123Client;
use serde_json::{Value, json};

use crate::mcp::protocol::ToolSpec;

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
}
