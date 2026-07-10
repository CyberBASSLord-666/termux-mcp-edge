//! Strict JSON-RPC 2.0 envelope validation for incoming MCP messages.
//!
//! This module separates malformed JSON from valid JSON that is not a valid MCP
//! request or notification. It intentionally performs no tool dispatch.

use serde_json::{Map, Value};

pub const JSON_RPC_VERSION: &str = "2.0";

#[derive(Debug, Clone, PartialEq)]
pub enum IncomingJsonRpcMessage {
    Request {
        id: Value,
        method: String,
        params: Option<Value>,
    },
    Notification {
        method: String,
        params: Option<Value>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonRpcEnvelopeError {
    ParseError {
        detail: String,
    },
    InvalidRequest {
        id: Option<Value>,
        reason: &'static str,
    },
}

pub fn parse_incoming_message(body: &[u8]) -> Result<IncomingJsonRpcMessage, JsonRpcEnvelopeError> {
    let value = serde_json::from_slice::<Value>(body).map_err(|error| {
        JsonRpcEnvelopeError::ParseError {
            detail: error.to_string(),
        }
    })?;

    let object = value
        .as_object()
        .ok_or_else(|| invalid(None, "JSON-RPC input must be an object."))?;
    validate_object(object)
}

fn validate_object(
    object: &Map<String, Value>,
) -> Result<IncomingJsonRpcMessage, JsonRpcEnvelopeError> {
    let detectable_id = object.get("id").and_then(valid_request_id).cloned();

    match object.get("jsonrpc") {
        Some(Value::String(version)) if version == JSON_RPC_VERSION => {}
        _ => {
            return Err(invalid(
                detectable_id,
                "JSON-RPC request must include jsonrpc exactly equal to 2.0.",
            ));
        }
    }

    let method = match object.get("method") {
        Some(Value::String(method)) => method.clone(),
        _ => {
            return Err(invalid(
                detectable_id,
                "JSON-RPC request must include a string method.",
            ));
        }
    };

    let params = object.get("params").cloned();
    if params
        .as_ref()
        .is_some_and(|params| !params.is_object() && !params.is_array())
    {
        return Err(invalid(
            detectable_id,
            "JSON-RPC params must be an object or array when present.",
        ));
    }

    match object.get("id") {
        None => Ok(IncomingJsonRpcMessage::Notification { method, params }),
        Some(id) => {
            let id = valid_request_id(id).cloned().ok_or_else(|| {
                invalid(
                    None,
                    "MCP request id must be a non-null string or integer number.",
                )
            })?;
            Ok(IncomingJsonRpcMessage::Request { id, method, params })
        }
    }
}

fn valid_request_id(value: &Value) -> Option<&Value> {
    match value {
        Value::String(_) => Some(value),
        Value::Number(number) if number.is_i64() || number.is_u64() => Some(value),
        _ => None,
    }
}

fn invalid(id: Option<Value>, reason: &'static str) -> JsonRpcEnvelopeError {
    JsonRpcEnvelopeError::InvalidRequest { id, reason }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn accepts_requests_with_string_or_integer_ids() {
        for value in [
            json!({"jsonrpc":"2.0","id":"request-1","method":"tools/list"}),
            json!({"jsonrpc":"2.0","id":7,"method":"tools/list"}),
            json!({"jsonrpc":"2.0","id":-7,"method":"tools/list"}),
        ] {
            let message = parse_incoming_message(value.to_string().as_bytes()).unwrap();
            assert!(matches!(message, IncomingJsonRpcMessage::Request { .. }));
        }
    }

    #[test]
    fn accepts_notifications_without_ids() {
        let value = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        });

        assert_eq!(
            parse_incoming_message(value.to_string().as_bytes()).unwrap(),
            IncomingJsonRpcMessage::Notification {
                method: "notifications/initialized".to_string(),
                params: Some(json!({})),
            }
        );
    }

    #[test]
    fn malformed_json_is_parse_error() {
        let error = parse_incoming_message(b"{not-json").unwrap_err();

        assert!(matches!(error, JsonRpcEnvelopeError::ParseError { .. }));
    }

    #[test]
    fn valid_non_object_json_is_invalid_request() {
        for body in ["null", "true", "1", "\"text\"", "[]", "[{}]"] {
            assert_eq!(
                parse_incoming_message(body.as_bytes()).unwrap_err(),
                JsonRpcEnvelopeError::InvalidRequest {
                    id: None,
                    reason: "JSON-RPC input must be an object.",
                }
            );
        }
    }

    #[test]
    fn requires_exact_json_rpc_version_and_string_method() {
        let cases = [
            json!({"id":1,"method":"tools/list"}),
            json!({"jsonrpc":"1.0","id":1,"method":"tools/list"}),
            json!({"jsonrpc":2.0,"id":1,"method":"tools/list"}),
            json!({"jsonrpc":"2.0","id":1}),
            json!({"jsonrpc":"2.0","id":1,"method":7}),
        ];

        for value in cases {
            let error = parse_incoming_message(value.to_string().as_bytes()).unwrap_err();
            assert!(matches!(
                error,
                JsonRpcEnvelopeError::InvalidRequest {
                    id: Some(Value::Number(_)),
                    ..
                }
            ));
        }
    }

    #[test]
    fn rejects_invalid_mcp_request_id_types_without_echoing_them() {
        for id in [json!(null), json!(true), json!([]), json!({}), json!(1.5)] {
            let value = json!({"jsonrpc":"2.0","id":id,"method":"tools/list"});
            let error = parse_incoming_message(value.to_string().as_bytes()).unwrap_err();

            assert!(matches!(
                error,
                JsonRpcEnvelopeError::InvalidRequest { id: None, .. }
            ));
        }
    }

    #[test]
    fn params_must_be_structured_when_present() {
        for params in [json!(null), json!(true), json!(1), json!("value")] {
            let value = json!({
                "jsonrpc":"2.0",
                "id":"params-test",
                "method":"tools/call",
                "params":params
            });
            let error = parse_incoming_message(value.to_string().as_bytes()).unwrap_err();

            assert!(matches!(
                error,
                JsonRpcEnvelopeError::InvalidRequest {
                    id: Some(Value::String(_)),
                    ..
                }
            ));
        }
    }
}
