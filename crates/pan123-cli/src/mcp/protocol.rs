use std::sync::Arc;

use pan123_sdk::CancelToken;
use serde_json::{Value, json};

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;

/// 解析一行 stdin。成功返回请求对象；失败返回一个已构造好的 JSON-RPC 错误响应。
pub fn parse_line(line: &str) -> std::result::Result<Value, Value> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Err(error_response(Value::Null, PARSE_ERROR, "empty message"));
    }
    let value: Value = match serde_json::from_str(trimmed) {
        Ok(value) => value,
        Err(err) => {
            return Err(error_response(
                Value::Null,
                PARSE_ERROR,
                &format!("invalid JSON: {err}"),
            ));
        }
    };
    let Some(object) = value.as_object() else {
        return Err(error_response(
            Value::Null,
            INVALID_REQUEST,
            "message must be a JSON object",
        ));
    };
    let id = object.get("id").cloned().unwrap_or(Value::Null);
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(error_response(
            id,
            INVALID_REQUEST,
            "jsonrpc must be \"2.0\"",
        ));
    }
    if object.get("method").and_then(Value::as_str).is_none() {
        return Err(error_response(id, INVALID_REQUEST, "method is required"));
    }
    Ok(value)
}

pub fn success_response(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

pub fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

pub const LATEST_PROTOCOL_VERSION: &str = "2025-06-18";
pub const SUPPORTED_PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

pub struct ToolCallOutcome {
    pub text: String,
    pub is_error: bool,
}

pub struct CallContext {
    pub progress: ProgressSink,
    pub cancel: CancelToken,
}

pub trait ToolHandler: Send + Sync + 'static {
    fn list_tools(&self) -> Vec<ToolSpec>;
    fn call(&self, name: &str, args: &Value, ctx: &CallContext) -> ToolCallOutcome;
}

/// 进度上报句柄。Task 6 补全真正的发送与节流逻辑。
#[derive(Clone)]
pub struct ProgressSink {
    inner: Option<Arc<ProgressChannel>>,
    token: Option<Value>,
}

struct ProgressChannel {
    tx: std::sync::mpsc::Sender<Value>,
    last: std::sync::Mutex<(u64, std::time::Instant)>,
}

impl ProgressSink {
    /// 不发任何通知的哑实现，用于测试与无 progressToken 的场景。
    pub fn discarding() -> Self {
        Self {
            inner: None,
            token: None,
        }
    }

    pub fn report(&self, _progress: u64, _total: Option<u64>, _message: Option<&str>) {}
}

/// 接收请求，返回应当写回的消息。`None` 表示这是通知，不应回复。
pub fn handle_request(
    handler: &dyn ToolHandler,
    request: &Value,
    ctx: &CallContext,
) -> Option<Value> {
    let id = request.get("id").cloned();
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    // 通知（无 id）：一律不回复。
    let id = id?;

    let response = match method.as_str() {
        "initialize" => {
            let requested = request
                .get("params")
                .and_then(|params| params.get("protocolVersion"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let negotiated = if SUPPORTED_PROTOCOL_VERSIONS.contains(&requested) {
                requested
            } else {
                LATEST_PROTOCOL_VERSION
            };
            success_response(
                id,
                json!({
                    "protocolVersion": negotiated,
                    "capabilities": { "tools": {} },
                    "serverInfo": {
                        "name": "pan123",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            )
        }
        "ping" => success_response(id, json!({})),
        "tools/list" => {
            let tools = handler
                .list_tools()
                .into_iter()
                .map(|spec| {
                    json!({
                        "name": spec.name,
                        "description": spec.description,
                        "inputSchema": spec.input_schema
                    })
                })
                .collect::<Vec<_>>();
            success_response(id, json!({ "tools": tools }))
        }
        "tools/call" => {
            let params = request.get("params").cloned().unwrap_or(Value::Null);
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(error_response(
                    id,
                    INVALID_PARAMS,
                    "tools/call requires params.name",
                ));
            };
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let outcome = handler.call(name, &arguments, ctx);
            success_response(
                id,
                json!({
                    "content": [ { "type": "text", "text": outcome.text } ],
                    "isError": outcome.is_error
                }),
            )
        }
        _ => error_response(id, METHOD_NOT_FOUND, &format!("method not found: {method}")),
    };
    Some(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_well_formed_request() {
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
        let request = parse_line(line).expect("should parse");
        assert_eq!(request["method"], "ping");
        assert_eq!(request["id"], 1);
    }

    #[test]
    fn rejects_non_json_with_parse_error() {
        let err = parse_line("not json at all").expect_err("should fail");
        assert_eq!(err["error"]["code"], PARSE_ERROR);
        assert_eq!(err["id"], Value::Null);
    }

    #[test]
    fn rejects_json_without_method() {
        let err = parse_line(r#"{"jsonrpc":"2.0","id":7}"#).expect_err("should fail");
        assert_eq!(err["error"]["code"], INVALID_REQUEST);
        assert_eq!(err["id"], 7);
    }

    #[test]
    fn rejects_wrong_jsonrpc_version() {
        let err =
            parse_line(r#"{"jsonrpc":"1.0","id":3,"method":"ping"}"#).expect_err("should fail");
        assert_eq!(err["error"]["code"], INVALID_REQUEST);
        assert_eq!(err["id"], 3);
    }

    #[test]
    fn rejects_non_object_json() {
        let err = parse_line("[1,2,3]").expect_err("should fail");
        assert_eq!(err["error"]["code"], INVALID_REQUEST);
    }

    #[test]
    fn parses_notification_without_id() {
        let line = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        let request = parse_line(line).expect("should parse");
        assert!(request.get("id").is_none());
    }

    #[test]
    fn builds_success_response() {
        let response = success_response(json!(5), json!({"ok": true}));
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 5);
        assert_eq!(response["result"]["ok"], true);
        assert!(response.get("error").is_none());
    }

    #[test]
    fn builds_error_response() {
        let response = error_response(json!(5), METHOD_NOT_FOUND, "nope");
        assert_eq!(response["id"], 5);
        assert_eq!(response["error"]["code"], METHOD_NOT_FOUND);
        assert_eq!(response["error"]["message"], "nope");
        assert!(response.get("result").is_none());
    }

    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    struct FakeHandler;

    impl ToolHandler for FakeHandler {
        fn list_tools(&self) -> Vec<ToolSpec> {
            vec![ToolSpec {
                name: "echo",
                description: "returns its input",
                input_schema: json!({
                    "type": "object",
                    "properties": { "text": { "type": "string" } },
                    "required": ["text"]
                }),
            }]
        }

        fn call(&self, name: &str, args: &Value, _ctx: &CallContext) -> ToolCallOutcome {
            if name != "echo" {
                return ToolCallOutcome {
                    text: format!("unknown tool: {name}"),
                    is_error: true,
                };
            }
            match args.get("text").and_then(Value::as_str) {
                Some(text) => ToolCallOutcome {
                    text: text.to_string(),
                    is_error: false,
                },
                None => ToolCallOutcome {
                    text: "text is required".into(),
                    is_error: true,
                },
            }
        }
    }

    fn ctx() -> CallContext {
        CallContext {
            progress: ProgressSink::discarding(),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    fn request(value: Value) -> Value {
        value
    }

    #[test]
    fn initialize_echoes_supported_version() {
        let handler = FakeHandler;
        let req = request(json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name":"x","version":"1"} }
        }));
        let response = handle_request(&handler, &req, &ctx()).expect("must respond");
        assert_eq!(response["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(response["result"]["serverInfo"]["name"], "pan123");
        assert!(response["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn initialize_falls_back_to_latest_version_when_unknown() {
        let handler = FakeHandler;
        let req = request(json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "1999-01-01" }
        }));
        let response = handle_request(&handler, &req, &ctx()).expect("must respond");
        assert_eq!(
            response["result"]["protocolVersion"],
            LATEST_PROTOCOL_VERSION
        );
    }

    #[test]
    fn tools_list_returns_declared_tools() {
        let handler = FakeHandler;
        let req = request(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));
        let response = handle_request(&handler, &req, &ctx()).expect("must respond");
        let tools = response["result"]["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "echo");
        assert!(tools[0]["inputSchema"].is_object());
    }

    #[test]
    fn tools_call_wraps_result_as_text_content() {
        let handler = FakeHandler;
        let req = request(json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "echo", "arguments": { "text": "hi" } }
        }));
        let response = handle_request(&handler, &req, &ctx()).expect("must respond");
        assert_eq!(response["result"]["content"][0]["type"], "text");
        assert_eq!(response["result"]["content"][0]["text"], "hi");
        assert_eq!(response["result"]["isError"], false);
    }

    #[test]
    fn tools_call_reports_tool_failure_with_is_error() {
        let handler = FakeHandler;
        let req = request(json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": { "name": "echo", "arguments": {} }
        }));
        let response = handle_request(&handler, &req, &ctx()).expect("must respond");
        assert_eq!(response["result"]["isError"], true);
        assert_eq!(response["result"]["content"][0]["text"], "text is required");
        assert!(
            response.get("error").is_none(),
            "工具级失败不得用 JSON-RPC error"
        );
    }

    #[test]
    fn unknown_method_returns_method_not_found() {
        let handler = FakeHandler;
        let req = request(json!({ "jsonrpc": "2.0", "id": 5, "method": "resources/list" }));
        let response = handle_request(&handler, &req, &ctx()).expect("must respond");
        assert_eq!(response["error"]["code"], METHOD_NOT_FOUND);
    }

    #[test]
    fn notification_gets_no_response() {
        let handler = FakeHandler;
        let req = request(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        assert!(handle_request(&handler, &req, &ctx()).is_none());
    }

    #[test]
    fn unknown_notification_is_silently_ignored() {
        let handler = FakeHandler;
        let req = request(json!({ "jsonrpc": "2.0", "method": "notifications/whatever" }));
        assert!(handle_request(&handler, &req, &ctx()).is_none());
    }

    #[test]
    fn ping_returns_empty_result() {
        let handler = FakeHandler;
        let req = request(json!({ "jsonrpc": "2.0", "id": 6, "method": "ping" }));
        let response = handle_request(&handler, &req, &ctx()).expect("must respond");
        assert!(response.get("result").is_some());
    }

    #[test]
    fn tools_call_without_name_is_invalid_params() {
        let handler = FakeHandler;
        let req = request(json!({
            "jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {}
        }));
        let response = handle_request(&handler, &req, &ctx()).expect("must respond");
        assert_eq!(response["error"]["code"], INVALID_PARAMS);
    }
}
