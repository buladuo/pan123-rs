use std::collections::HashMap;
use std::io::{BufRead, Read, Write};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};

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
    pub fn new(tx: Sender<Value>, token: Option<Value>) -> Self {
        Self {
            inner: Some(Arc::new(ProgressChannel {
                tx,
                last: Mutex::new((0, std::time::Instant::now())),
            })),
            token,
        }
    }

    /// 不发任何通知的哑实现，用于测试与无 progressToken 的场景。
    pub fn discarding() -> Self {
        Self {
            inner: None,
            token: None,
        }
    }

    /// 按「进度变化 ≥1% 或距上次 ≥500ms」节流后上报。没有 progressToken 时直接丢弃。
    pub fn report(&self, progress: u64, total: Option<u64>, message: Option<&str>) {
        let (Some(channel), Some(token)) = (self.inner.as_ref(), self.token.as_ref()) else {
            return;
        };
        {
            let mut last = match channel.last.lock() {
                Ok(guard) => guard,
                Err(_) => return,
            };
            let step_ok = match total {
                Some(total) if total > 0 => {
                    let last_percent = last.0.saturating_mul(100) / total;
                    let now_percent = progress.saturating_mul(100) / total;
                    now_percent > last_percent
                }
                _ => false,
            };
            let time_ok = last.1.elapsed() >= std::time::Duration::from_millis(500);
            if !step_ok && !time_ok && progress != 0 {
                return;
            }
            *last = (progress, std::time::Instant::now());
        }
        let _ = channel.tx.send(json!({
            "jsonrpc": "2.0",
            "method": "notifications/progress",
            "params": {
                "progressToken": token,
                "progress": progress,
                "total": total,
                "message": message
            }
        }));
    }
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

pub const MAX_CONCURRENT_CALLS: usize = 4;

/// 单行上限，防止畸形客户端用超大行拖垮进程。
const MAX_LINE_BYTES: u64 = 8 * 1024 * 1024;

pub fn run_server<R: BufRead, W: Write + Send + 'static>(
    handler: Arc<dyn ToolHandler>,
    mut reader: R,
    writer: W,
) -> std::io::Result<()> {
    let (tx, rx) = mpsc::channel::<Value>();

    // 写线程独占 writer，串行写出每一行。
    let writer_handle = std::thread::spawn(move || {
        let mut writer = writer;
        for message in rx {
            if serde_json::to_writer(&mut writer, &message).is_err() {
                break;
            }
            if writer.write_all(b"\n").is_err() || writer.flush().is_err() {
                break;
            }
        }
    });

    let pending: Arc<Mutex<HashMap<String, CancelToken>>> = Arc::new(Mutex::new(HashMap::new()));
    let active_workers: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
    let mut worker_handles = Vec::new();
    let mut line = String::new();
    let mut initialized = false;

    loop {
        line.clear();
        let read = (&mut reader).take(MAX_LINE_BYTES).read_line(&mut line)?;
        if read == 0 {
            break; // EOF：客户端关闭，干净退出。
        }
        let parse_result = parse_line(&line);
        let request = match parse_result {
            Ok(request) => request,
            Err(error_message) => {
                let _ = tx.send(error_message);
                continue;
            }
        };

        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        // 生命周期状态机：initialize 之前只接受 initialize 本身。
        if method != "initialize" && !initialized {
            if let Some(id) = request.get("id").cloned() {
                let _ = tx.send(error_response(id, INVALID_REQUEST, "必须先调用 initialize"));
            }
            continue; // 无 id 的通知在此静默忽略
        }
        if method == "initialize" {
            initialized = true;
        }

        if method == "notifications/cancelled" {
            let key = request
                .get("params")
                .and_then(|params| params.get("requestId"))
                .map(id_key);
            if let Some(key) = key
                && let Ok(guard) = pending.lock()
                && let Some(token) = guard.get(&key)
            {
                token.store(true, Ordering::SeqCst);
            }
            continue;
        }

        let progress_token = request
            .get("params")
            .and_then(|params| params.get("_meta"))
            .and_then(|meta| meta.get("progressToken"))
            .cloned();
        let write_tx = tx.clone();
        let handler_for_worker = Arc::clone(&handler);
        let is_tool_call = method == "tools/call";
        let request_id = request.get("id").cloned();
        let pending_for_worker = Arc::clone(&pending);
        let active_for_worker = Arc::clone(&active_workers);

        // initialize / tools/list / ping 就地处理；tools/call 派给工作线程。
        let ctx = CallContext {
            progress: ProgressSink::new(write_tx.clone(), progress_token),
            cancel: Arc::new(AtomicBool::new(false)),
        };

        if !is_tool_call {
            if let Some(response) = handle_request(handler_for_worker.as_ref(), &request, &ctx) {
                let _ = write_tx.send(response);
            }
            continue;
        }

        let Some(id) = request_id else {
            continue; // tools/call 作为通知：无 id，忽略。
        };
        let key = id_key(&id);

        if active_for_worker.fetch_add(1, Ordering::SeqCst) >= MAX_CONCURRENT_CALLS {
            active_for_worker.fetch_sub(1, Ordering::SeqCst);
            let _ = write_tx.send(success_response(
                id,
                json!({
                    "content": [{
                        "type": "text",
                        "text": format!("并发上限为 {MAX_CONCURRENT_CALLS}，请稍后重试")
                    }],
                    "isError": true
                }),
            ));
            continue;
        }

        let cancel = ctx.cancel.clone();
        if let Ok(mut guard) = pending.lock() {
            guard.insert(key.clone(), cancel.clone());
        }

        let handle = std::thread::Builder::new()
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    handle_request(handler_for_worker.as_ref(), &request, &ctx)
                }));
                let cancelled = cancel.load(Ordering::SeqCst);
                if !cancelled {
                    let response = match result {
                        Ok(Some(response)) => Some(response),
                        Ok(None) => None,
                        Err(_) => Some(success_response(
                            id,
                            json!({
                                "content": [{ "type": "text", "text": "工具执行时发生内部错误" }],
                                "isError": true
                            }),
                        )),
                    };
                    if let Some(response) = response {
                        let _ = write_tx.send(response);
                    }
                }
                if let Ok(mut guard) = pending_for_worker.lock() {
                    guard.remove(&key);
                }
                active_for_worker.fetch_sub(1, Ordering::SeqCst);
            })
            .expect("failed to spawn tool worker thread");
        worker_handles.push(handle);
    }

    for handle in worker_handles {
        let _ = handle.join();
    }
    drop(tx);
    let _ = writer_handle.join();
    Ok(())
}

/// 请求 id 归一化为字符串键：JSON 数字与字符串形式视为同一个 id。
fn id_key(id: &Value) -> String {
    match id {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
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

    use std::io::{BufReader, Cursor};
    use std::sync::Mutex;
    use std::time::Duration;

    /// 记录每次 call 的线程 id，用于断言并发。
    struct RecordingHandler {
        seen: Mutex<Vec<std::thread::ThreadId>>,
        delay_ms: u64,
    }

    impl ToolHandler for RecordingHandler {
        fn list_tools(&self) -> Vec<ToolSpec> {
            vec![ToolSpec {
                name: "slow",
                description: "sleeps",
                input_schema: json!({ "type": "object" }),
            }]
        }

        fn call(&self, _name: &str, _args: &Value, ctx: &CallContext) -> ToolCallOutcome {
            self.seen.lock().unwrap().push(std::thread::current().id());
            let deadline = std::time::Instant::now() + Duration::from_millis(self.delay_ms);
            while std::time::Instant::now() < deadline {
                if ctx.cancel.load(std::sync::atomic::Ordering::SeqCst) {
                    return ToolCallOutcome {
                        text: "cancelled".into(),
                        is_error: true,
                    };
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            ToolCallOutcome {
                text: "done".into(),
                is_error: false,
            }
        }
    }

    fn run_with_input(handler: Arc<dyn ToolHandler>, input: &str) -> String {
        let reader = BufReader::new(Cursor::new(input.as_bytes().to_vec()));
        let output = Vec::new();
        let sink = Arc::new(Mutex::new(output));
        let sink_writer = SharedVec(Arc::clone(&sink));
        run_server(handler, reader, sink_writer).expect("server should exit cleanly");
        let bytes = sink.lock().unwrap().clone();
        String::from_utf8(bytes).expect("utf8")
    }

    #[derive(Clone)]
    struct SharedVec(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for SharedVec {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn response_lines(output: &str) -> Vec<Value> {
        output
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).expect("each stdout line must be valid JSON"))
            .collect()
    }

    /// 一次合法的 initialize，供需要「已初始化会话」的测试前置使用。
    const INITIALIZE_LINE: &str = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#;

    /// 前置一次 initialize，返回其后的响应行（已丢弃 initialize 自己的那条）。
    fn run_session(handler: Arc<dyn ToolHandler>, body: &str) -> Vec<Value> {
        let mut input = String::from(INITIALIZE_LINE);
        input.push('\n');
        input.push_str(body);
        let output = run_with_input(handler, &input);
        let mut lines = response_lines(&output);
        assert_eq!(
            lines.first().map(|line| line["id"].clone()),
            Some(json!(0)),
            "首条响应应来自 initialize"
        );
        lines.remove(0);
        lines
    }

    #[test]
    fn eof_produces_no_panic_and_no_output() {
        let handler: Arc<dyn ToolHandler> = Arc::new(FakeHandler);
        let output = run_with_input(handler, "");
        assert!(output.trim().is_empty());
    }

    #[test]
    fn eof_while_call_in_flight_still_writes_its_response() {
        let handler = Arc::new(RecordingHandler {
            seen: Mutex::new(Vec::new()),
            delay_ms: 150,
        });
        // 输入在派发调用后立刻结束：主线程读到 EOF 后必须等在工作线程上，
        // 而不是直接丢弃它——否则客户端会拿到一个永不到来的响应。
        let body = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"slow","arguments":{}}}"#,
            "\n",
        );
        let lines = run_session(handler, body);
        assert_eq!(lines.len(), 1, "在途调用必须完成并写出响应");
        assert_eq!(lines[0]["id"], 1);
    }

    #[test]
    fn request_before_initialize_is_rejected() {
        let handler: Arc<dyn ToolHandler> = Arc::new(FakeHandler);
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#,
            "\n",
        );
        let output = run_with_input(handler, input);
        let lines = response_lines(&output);
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0]["error"]["code"], INVALID_REQUEST,
            "initialize 之前的请求必须被拒绝"
        );
        assert_eq!(lines[1]["id"], 2);
        assert!(
            lines[2]["result"]["tools"].is_array(),
            "initialize 之后必须恢复正常服务"
        );
    }

    #[test]
    fn handles_a_full_session_in_order() {
        let handler: Arc<dyn ToolHandler> = Arc::new(FakeHandler);
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"echo","arguments":{"text":"hi"}}}"#,
            "\n",
        );
        let output = run_with_input(handler, input);
        let lines = response_lines(&output);
        assert_eq!(lines.len(), 3, "通知不应产生响应行");
        assert_eq!(lines[0]["id"], 1);
        assert_eq!(lines[1]["id"], 2);
        assert_eq!(lines[2]["id"], 3);
        assert_eq!(lines[2]["result"]["content"][0]["text"], "hi");
    }

    #[test]
    fn malformed_line_does_not_stop_the_server() {
        let handler: Arc<dyn ToolHandler> = Arc::new(FakeHandler);
        let body = concat!(
            "not json\n",
            r#"{"jsonrpc":"2.0","id":9,"method":"ping"}"#,
            "\n",
        );
        let lines = run_session(handler, body);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["error"]["code"], PARSE_ERROR);
        assert_eq!(lines[1]["id"], 9, "畸形输入之后仍须继续服务");
        assert!(lines[1]["result"].is_object(), "后续请求应被正常处理");
    }

    #[test]
    fn oversized_single_line_is_rejected_without_panicking() {
        let handler: Arc<dyn ToolHandler> = Arc::new(FakeHandler);
        let mut body = String::from(r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":{"x":""#);
        body.push_str(&"a".repeat(2 * 1024 * 1024));
        body.push_str("\"}}\n");
        let lines = run_session(handler, &body);
        assert_eq!(lines.len(), 1, "超大行应被当作一次请求处理");
        assert!(lines[0]["result"].is_object() || lines[0]["error"].is_object());
    }

    #[test]
    fn concurrent_calls_run_on_different_threads() {
        let handler = Arc::new(RecordingHandler {
            seen: Mutex::new(Vec::new()),
            delay_ms: 200,
        });
        let body = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"slow","arguments":{}}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"slow","arguments":{}}}"#,
            "\n",
        );
        let lines = run_session(handler.clone(), body);
        assert_eq!(lines.len(), 2);
        let seen = handler.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_ne!(seen[0], seen[1], "两个调用应在不同线程上执行");
    }

    #[test]
    fn cancelled_call_gets_no_response() {
        let handler = Arc::new(RecordingHandler {
            seen: Mutex::new(Vec::new()),
            delay_ms: 5_000,
        });
        let body = concat!(
            r#"{"jsonrpc":"2.0","id":42,"method":"tools/call","params":{"name":"slow","arguments":{}}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":42,"reason":"user"}}"#,
            "\n",
        );
        let lines = run_session(handler, body);
        assert!(
            lines.is_empty(),
            "被取消的请求不应返回任何响应，实际：{lines:?}"
        );
    }

    #[test]
    fn cancelled_unknown_id_is_ignored() {
        let handler: Arc<dyn ToolHandler> = Arc::new(FakeHandler);
        let body = concat!(
            r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":9999}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
            "\n",
        );
        let lines = run_session(handler, body);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["id"], 1);
        assert!(lines[0]["result"].is_object(), "后续请求不应受影响");
    }

    #[test]
    fn cancelled_running_call_does_not_block_later_requests() {
        let handler = Arc::new(RecordingHandler {
            seen: Mutex::new(Vec::new()),
            delay_ms: 400,
        });
        let body = concat!(
            r#"{"jsonrpc":"2.0","id":42,"method":"tools/call","params":{"name":"slow","arguments":{}}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":42}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":43,"method":"ping"}"#,
            "\n",
        );
        let lines = run_session(handler, body);
        assert_eq!(lines.len(), 1, "只有 ping 应有响应，被取消的调用不应有");
        assert_eq!(lines[0]["id"], 43);
    }

    #[test]
    fn exceeded_concurrency_returns_is_error() {
        let handler = Arc::new(RecordingHandler {
            seen: Mutex::new(Vec::new()),
            delay_ms: 500,
        });
        let mut body = String::new();
        for id in 1..=6 {
            body.push_str(&format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"tools/call\",\"params\":{{\"name\":\"slow\",\"arguments\":{{}}}}}}\n"
            ));
        }
        let lines = run_session(handler, &body);
        assert_eq!(lines.len(), 6, "每个请求都必须有一条响应");
        let rejected = lines
            .iter()
            .filter(|line| line["result"]["isError"] == json!(true))
            .count();
        assert!(rejected >= 2, "超出并发上限的请求应被拒绝，实际 {rejected}");
    }
}
