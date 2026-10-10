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
}
