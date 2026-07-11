//! Strict JSON-RPC 2.0 envelope validation for incoming MCP messages.
//!
//! This module separates malformed JSON from valid JSON that is not a valid MCP
//! request, notification, or response. It intentionally performs no dispatch.

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
    Response,
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

    if object.contains_key("method") {
        validate_request_or_notification(object, detectable_id)
    } else {
        validate_response(object, detectable_id)
    }
}

fn validate_request_or_notification(
    object: &Map<String, Value>,
    detectable_id: Option<Value>,
) -> Result<IncomingJsonRpcMessage, JsonRpcEnvelopeError> {
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
    if params.as_ref().is_some_and(|params| !params.is_object()) {
        return Err(invalid(
            detectable_id,
            "MCP params must be an object when present.",
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

fn validate_response(
    object: &Map<String, Value>,
    detectable_id: Option<Value>,
) -> Result<IncomingJsonRpcMessage, JsonRpcEnvelopeError> {
    if object.contains_key("params") {
        return Err(invalid(
            detectable_id,
            "JSON-RPC responses must not include params.",
        ));
    }

    match (object.get("result"), object.get("error")) {
        (Some(result), None) => {
            if !result.is_object() {
                return Err(invalid(
                    detectable_id,
                    "MCP success responses must include an object result.",
                ));
            }
            if object.get("id").and_then(valid_request_id).is_none() {
                return Err(invalid(
                    None,
                    "MCP success responses must include a valid request id.",
                ));
            }
        }
        (None, Some(Value::Object(error))) => {
            if object
                .get("id")
                .is_some_and(|id| valid_request_id(id).is_none())
            {
                return Err(invalid(
                    None,
                    "MCP error response id must be a string or integer when present.",
                ));
            }
            if !error.get("code").is_some_and(is_integer_number)
                || !error.get("message").is_some_and(Value::is_string)
            {
                return Err(invalid(
                    detectable_id,
                    "MCP error responses require an integer code and string message.",
                ));
            }
        }
        (None, Some(_)) => {
            return Err(invalid(
                detectable_id,
                "MCP error responses must include an object error.",
            ));
        }
        _ => {
            return Err(invalid(
                detectable_id,
                "JSON-RPC responses must include exactly one of result or error.",
            ));
        }
    }

    Ok(IncomingJsonRpcMessage::Response)
}

fn valid_request_id(value: &Value) -> Option<&Value> {
    match value {
        Value::String(_) => Some(value),
        Value::Number(number) if number.is_i64() || number.is_u64() => Some(value),
        _ => None,
    }
}

fn is_integer_number(value: &Value) -> bool {
    value
        .as_number()
        .is_some_and(|number| number.is_i64() || number.is_u64())
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
    fn accepts_success_and_error_responses() {
        for value in [
            json!({"jsonrpc":"2.0","id":"request-1","result":{}}),
            json!({"jsonrpc":"2.0","id":7,"error":{"code":-32603,"message":"failed"}}),
            json!({"jsonrpc":"2.0","error":{"code":-32600,"message":"invalid"}}),
        ] {
            assert_eq!(
                parse_incoming_message(value.to_string().as_bytes()).unwrap(),
                IncomingJsonRpcMessage::Response
            );
        }
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
    fn mcp_params_must_be_objects_when_present() {
        for params in [
            json!(null),
            json!(true),
            json!(1),
            json!("value"),
            json!([]),
        ] {
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

    #[test]
    fn rejects_malformed_or_ambiguous_responses() {
        let cases = [
            json!({"jsonrpc":"2.0","id":1}),
            json!({"jsonrpc":"2.0","id":1,"result":null}),
            json!({"jsonrpc":"2.0","result":{}}),
            json!({"jsonrpc":"2.0","id":null,"result":{}}),
            json!({"jsonrpc":"2.0","id":1,"result":{},"error":{"code":-1,"message":"no"}}),
            json!({"jsonrpc":"2.0","id":1,"error":null}),
            json!({"jsonrpc":"2.0","id":1,"error":{"code":-1.5,"message":"no"}}),
            json!({"jsonrpc":"2.0","id":1,"error":{"code":-1,"message":7}}),
            json!({"jsonrpc":"2.0","id":1,"result":{},"params":{}}),
        ];

        for value in cases {
            assert!(matches!(
                parse_incoming_message(value.to_string().as_bytes()),
                Err(JsonRpcEnvelopeError::InvalidRequest { .. })
            ));
        }
    }
}
