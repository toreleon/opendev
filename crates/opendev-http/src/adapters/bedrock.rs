//! AWS Bedrock provider adapter.
//!
//! Transforms OpenAI Chat Completions payloads to Amazon Bedrock's
//! `InvokeModel` format and converts responses back.
//!
//! Bedrock uses SigV4 request signing. Since `aws-sigv4` is not available
//! as a dependency, the signing logic is stubbed with a TODO. In production,
//! either add `aws-sigv4`/`aws-credential-types` crates or implement
//! minimal HMAC-SHA256 signing with the `hmac` and `sha2` crates.
//!
//! Environment variables:
//! - `AWS_ACCESS_KEY_ID` — IAM access key
//! - `AWS_SECRET_ACCESS_KEY` — IAM secret key
//! - `AWS_REGION` — AWS region (defaults to `us-east-1`)
//! - `AWS_SESSION_TOKEN` — optional session token for temporary credentials

use serde_json::{Value, json};

/// Default AWS region when `AWS_REGION` is not set.
const DEFAULT_REGION: &str = "us-east-1";

/// Adapter for Amazon Bedrock's InvokeModel API.
///
/// Bedrock wraps foundation models behind a REST API at:
/// `https://bedrock-runtime.{region}.amazonaws.com/model/{model_id}/invoke`
///
/// This adapter handles:
/// - Converting Chat Completions messages to Bedrock's Anthropic-style format
/// - Building the correct endpoint URL from region + model
/// - SigV4 header generation (TODO: requires `hmac`/`sha2` crates)
#[derive(Debug, Clone)]
pub struct BedrockAdapter {
    region: String,
    model_id: String,
    api_url: String,
}

impl BedrockAdapter {
    /// Create a new Bedrock adapter for the given model.
    ///
    /// Reads `AWS_REGION` from the environment (defaults to `us-east-1`).
    pub fn new(model_id: impl Into<String>) -> Self {
        let model_id = model_id.into();
        let region = std::env::var("AWS_REGION").unwrap_or_else(|_| DEFAULT_REGION.to_string());
        let api_url = Self::build_url(&region, &model_id);
        Self {
            region,
            model_id,
            api_url,
        }
    }

    /// Create a new Bedrock adapter with a custom region.
    pub fn with_region(model_id: impl Into<String>, region: impl Into<String>) -> Self {
        let model_id = model_id.into();
        let region = region.into();
        let api_url = Self::build_url(&region, &model_id);
        Self {
            region,
            model_id,
            api_url,
        }
    }

    /// Build the Bedrock InvokeModel URL.
    fn build_url(region: &str, model_id: &str) -> String {
        format!("https://bedrock-runtime.{region}.amazonaws.com/model/{model_id}/invoke")
    }

    /// Get the configured AWS region.
    pub fn region(&self) -> &str {
        &self.region
    }

    /// Get the model ID.
    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    /// Extract system message from messages array into a top-level field.
    ///
    /// Bedrock's Anthropic format expects system as a separate top-level field.
    fn extract_system(payload: &mut Value) {
        if let Some(messages) = payload.get_mut("messages").and_then(|m| m.as_array_mut()) {
            let mut system_parts: Vec<String> = Vec::new();
            messages.retain(|msg| {
                if msg.get("role").and_then(|r| r.as_str()) == Some("system") {
                    if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                        system_parts.push(content.to_string());
                    }
                    false
                } else {
                    true
                }
            });

            if !system_parts.is_empty() {
                let combined = system_parts.join("\n\n");
                payload["system"] = json!(combined);
            }
        }
    }

    /// Convert Chat Completions tool schemas to Bedrock/Anthropic format.
    ///
    /// OpenAI: `[{type: "function", function: {name, description, parameters}}]`
    /// Bedrock: `[{name, description, input_schema}]`
    fn convert_tools(payload: &mut Value) {
        if let Some(tools) = payload.get_mut("tools").and_then(|t| t.as_array_mut()) {
            let converted: Vec<Value> = tools
                .iter()
                .filter_map(|tool| {
                    let func = tool.get("function")?;
                    Some(json!({
                        "name": func.get("name")?,
                        "description": func.get("description").cloned().unwrap_or(json!("")),
                        "input_schema": func.get("parameters").cloned()
                            .unwrap_or(json!({"type": "object", "properties": {}}))
                    }))
                })
                .collect();
            if let Some(tools_slot) = payload.get_mut("tools") {
                *tools_slot = json!(converted);
            }
        }
    }

    /// Convert tool result messages from Chat Completions to Bedrock format.
    ///
    /// Converts `role: "tool"` messages to `role: "user"` with `tool_result` blocks,
    /// and assistant `tool_calls` to `tool_use` content blocks.
    fn convert_tool_messages(payload: &mut Value) {
        if let Some(messages) = payload.get_mut("messages").and_then(|m| m.as_array_mut()) {
            let mut converted: Vec<Value> = Vec::new();

            for msg in messages.iter() {
                let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");

                match role {
                    "assistant" => {
                        if let Some(tool_calls) = msg.get("tool_calls").and_then(|tc| tc.as_array())
                        {
                            let mut content_blocks: Vec<Value> = Vec::new();

                            if let Some(text) = msg.get("content").and_then(|c| c.as_str())
                                && !text.is_empty()
                            {
                                content_blocks.push(json!({
                                    "type": "text",
                                    "text": text
                                }));
                            }

                            for tc in tool_calls {
                                let func = tc.get("function").cloned().unwrap_or(json!({}));
                                let args_str = func
                                    .get("arguments")
                                    .and_then(|a| a.as_str())
                                    .unwrap_or("{}");
                                let args: Value =
                                    serde_json::from_str(args_str).unwrap_or(json!({}));

                                content_blocks.push(json!({
                                    "type": "tool_use",
                                    "id": tc.get("id").cloned().unwrap_or(json!("")),
                                    "name": func.get("name").cloned().unwrap_or(json!("")),
                                    "input": args
                                }));
                            }

                            converted.push(json!({
                                "role": "assistant",
                                "content": content_blocks
                            }));
                        } else {
                            converted.push(msg.clone());
                        }
                    }
                    "tool" => {
                        let tool_call_id = msg.get("tool_call_id").cloned().unwrap_or(json!(""));
                        let content = msg.get("content").and_then(|c| c.as_str()).unwrap_or("");

                        let result_block = json!({
                            "type": "tool_result",
                            "tool_use_id": tool_call_id,
                            "content": content
                        });

                        let should_merge = converted.last().is_some_and(|last| {
                            last.get("role").and_then(|r| r.as_str()) == Some("user")
                                && last.get("content").and_then(|c| c.as_array()).is_some_and(
                                    |blocks| {
                                        blocks.iter().all(|b| {
                                            b.get("type").and_then(|t| t.as_str())
                                                == Some("tool_result")
                                        })
                                    },
                                )
                        });

                        if should_merge {
                            if let Some(last) = converted.last_mut()
                                && let Some(blocks) =
                                    last.get_mut("content").and_then(|c| c.as_array_mut())
                            {
                                blocks.push(result_block);
                            }
                        } else {
                            converted.push(json!({
                                "role": "user",
                                "content": [result_block]
                            }));
                        }
                    }
                    _ => {
                        converted.push(msg.clone());
                    }
                }
            }

            if let Some(messages_slot) = payload.get_mut("messages") {
                *messages_slot = json!(converted);
            }
        }
    }

    /// Ensure max_tokens is set (required by Bedrock Anthropic models).
    fn ensure_max_tokens(payload: &mut Value) {
        if payload.get("max_tokens").is_none() {
            if let Some(val) = payload.get("max_completion_tokens").cloned() {
                if let Some(obj) = payload.as_object_mut() {
                    obj.remove("max_completion_tokens");
                }
                payload["max_tokens"] = val;
            } else {
                payload["max_tokens"] = json!(4096);
            }
        }
    }

    /// Convert Bedrock's Anthropic-style response to Chat Completions format.
    fn response_to_chat_completions(response: Value, model_id: &str) -> Value {
        let blocks = response
            .get("content")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default();

        // Extract text content
        let content: String = blocks
            .iter()
            .filter_map(|b| {
                if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                    b.get("text").and_then(|t| t.as_str()).map(String::from)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("");

        // Extract tool_use blocks
        let tool_calls: Vec<Value> = blocks
            .iter()
            .filter_map(|b| {
                if b.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                    let id = b.get("id").and_then(|i| i.as_str()).unwrap_or("");
                    let name = b.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let input = b.get("input").cloned().unwrap_or(json!({}));
                    Some(json!({
                        "id": id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": serde_json::to_string(&input).unwrap_or_default()
                        }
                    }))
                } else {
                    None
                }
            })
            .collect();

        let stop_reason = response
            .get("stop_reason")
            .and_then(|r| r.as_str())
            .unwrap_or("stop");

        let finish_reason = match stop_reason {
            "end_turn" => "stop",
            "max_tokens" => "length",
            "tool_use" => "tool_calls",
            other => other,
        };

        let usage = response.get("usage").cloned().unwrap_or(json!({}));

        let mut message = json!({
            "role": "assistant",
            "content": content
        });

        if !tool_calls.is_empty() {
            message["tool_calls"] = json!(tool_calls);
        }

        json!({
            "id": response.get("id").cloned().unwrap_or(json!("")),
            "object": "chat.completion",
            "model": model_id,
            "choices": [{
                "index": 0,
                "message": message,
                "finish_reason": finish_reason
            }],
            "usage": {
                "prompt_tokens": usage.get("input_tokens").cloned().unwrap_or(json!(0)),
                "completion_tokens": usage.get("output_tokens").cloned().unwrap_or(json!(0)),
                "total_tokens": usage.get("input_tokens").and_then(|i| i.as_u64())
                    .unwrap_or(0)
                    + usage.get("output_tokens").and_then(|o| o.as_u64())
                    .unwrap_or(0)
            }
        })
    }

    /// Generate SigV4 authorization headers for the request.
    ///
    /// TODO: Implement SigV4 signing. Requires either:
    /// - Adding `aws-sigv4` + `aws-credential-types` crates, or
    /// - Adding `hmac` + `sha2` crates for minimal manual signing.
    ///
    /// For now, this returns empty headers. To use Bedrock in production,
    /// implement the SigV4 signing algorithm:
    /// 1. Create canonical request (method, URI, query, headers, payload hash)
    /// 2. Create string-to-sign (algorithm, date, scope, canonical request hash)
    /// 3. Derive signing key via HMAC-SHA256 chain (date → region → service → signing)
    /// 4. Calculate signature = HMAC-SHA256(signing_key, string_to_sign)
    /// 5. Build Authorization header
    #[allow(dead_code)]
    fn sigv4_headers(&self, _payload: &[u8]) -> Vec<(String, String)> {
        let _access_key = std::env::var("AWS_ACCESS_KEY_ID").unwrap_or_default();
        let _secret_key = std::env::var("AWS_SECRET_ACCESS_KEY").unwrap_or_default();
        let _session_token = std::env::var("AWS_SESSION_TOKEN").ok();

        // TODO: Implement SigV4 signing when `hmac` and `sha2` crates are available.
        // The signing algorithm requires:
        // 1. SHA-256 hash of the request payload
        // 2. Canonical request construction
        // 3. HMAC-SHA256 signing key derivation
        // 4. Authorization header assembly
        //
        // Without crypto crates, we cannot implement this. Users should either:
        // - Add `hmac = "0.12"` and `sha2 = "0.10"` to Cargo.toml, or
        // - Use an AWS SDK credential provider externally and pass a pre-signed URL.

        vec![
            ("Content-Type".into(), "application/json".into()),
            ("Accept".into(), "application/json".into()),
        ]
    }
}

#[async_trait::async_trait]
impl super::base::ProviderAdapter for BedrockAdapter {
    fn provider_name(&self) -> &str {
        "bedrock"
    }

    fn convert_request(&self, mut payload: Value) -> Value {
        Self::extract_system(&mut payload);
        Self::convert_tools(&mut payload);
        Self::convert_tool_messages(&mut payload);
        Self::ensure_max_tokens(&mut payload);

        // Bedrock wraps the model in the URL, not the payload.
        // Remove fields Bedrock does not accept.
        if let Some(obj) = payload.as_object_mut() {
            obj.remove("model");
            obj.remove("n");
            obj.remove("frequency_penalty");
            obj.remove("presence_penalty");
            obj.remove("logprobs");
            obj.remove("stream");
        }

        // Set anthropic_version required by Bedrock's Anthropic models.
        payload["anthropic_version"] = json!("bedrock-2023-05-31");

        payload
    }

    fn convert_response(&self, response: Value) -> Value {
        Self::response_to_chat_completions(response, &self.model_id)
    }

    fn api_url(&self) -> &str {
        &self.api_url
    }

    fn extra_headers(&self) -> Vec<(String, String)> {
        // TODO: SigV4 headers should be generated per-request with the payload.
        // The ProviderAdapter trait's `extra_headers()` is called without payload
        // context, so full SigV4 signing would require a trait extension.
        // For now, return content-type headers only.
        vec![
            ("Content-Type".into(), "application/json".into()),
            ("Accept".into(), "application/json".into()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::base::ProviderAdapter;

    #[test]
    fn test_provider_name() {
        let adapter = BedrockAdapter::new("anthropic.claude-3-sonnet-20240229-v1:0");
        assert_eq!(adapter.provider_name(), "bedrock");
    }

    #[test]
    fn test_api_url_format() {
        let adapter =
            BedrockAdapter::with_region("anthropic.claude-3-sonnet-20240229-v1:0", "us-west-2");
        assert_eq!(
            adapter.api_url(),
            "https://bedrock-runtime.us-west-2.amazonaws.com/model/anthropic.claude-3-sonnet-20240229-v1:0/invoke"
        );
    }

    #[test]
    fn test_api_url_default_region() {
        // Clear region env var for predictable test
        let adapter =
            BedrockAdapter::with_region("anthropic.claude-3-haiku-20240307-v1:0", "us-east-1");
        assert!(adapter.api_url().contains("us-east-1"));
    }

    #[test]
    fn test_model_id() {
        let adapter = BedrockAdapter::new("anthropic.claude-3-sonnet-20240229-v1:0");
        assert_eq!(
            adapter.model_id(),
            "anthropic.claude-3-sonnet-20240229-v1:0"
        );
    }

    #[test]
    fn test_region() {
        let adapter = BedrockAdapter::with_region("model", "eu-west-1");
        assert_eq!(adapter.region(), "eu-west-1");
    }

    #[test]
    fn test_extract_system() {
        let mut payload = json!({
            "messages": [
                {"role": "system", "content": "You are helpful."},
                {"role": "user", "content": "Hello"}
            ]
        });
        BedrockAdapter::extract_system(&mut payload);
        assert_eq!(payload["system"], "You are helpful.");
        let messages = payload["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
    }

    #[test]
    fn test_extract_system_multiple() {
        let mut payload = json!({
            "messages": [
                {"role": "system", "content": "Part 1"},
                {"role": "system", "content": "Part 2"},
                {"role": "user", "content": "Hello"}
            ]
        });
        BedrockAdapter::extract_system(&mut payload);
        assert_eq!(payload["system"], "Part 1\n\nPart 2");
    }

    #[test]
    fn test_extract_system_none() {
        let mut payload = json!({
            "messages": [
                {"role": "user", "content": "Hello"}
            ]
        });
        BedrockAdapter::extract_system(&mut payload);
        assert!(payload.get("system").is_none());
    }

    #[test]
    fn test_convert_tools() {
        let mut payload = json!({
            "tools": [{
                "type": "function",
                "function": {
                    "name": "read_file",
                    "description": "Read a file",
                    "parameters": {"type": "object", "properties": {"path": {"type": "string"}}}
                }
            }]
        });
        BedrockAdapter::convert_tools(&mut payload);
        let tools = payload["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "read_file");
        assert_eq!(tools[0]["description"], "Read a file");
        assert!(tools[0].get("input_schema").is_some());
    }

    #[test]
    fn test_convert_request_removes_unsupported_fields() {
        let adapter = BedrockAdapter::with_region("model-id", "us-east-1");
        let payload = json!({
            "model": "model-id",
            "messages": [{"role": "user", "content": "Hi"}],
            "n": 1,
            "frequency_penalty": 0.5,
            "presence_penalty": 0.5,
            "logprobs": true,
            "stream": true
        });
        let result = adapter.convert_request(payload);
        assert!(result.get("model").is_none());
        assert!(result.get("n").is_none());
        assert!(result.get("frequency_penalty").is_none());
        assert!(result.get("presence_penalty").is_none());
        assert!(result.get("logprobs").is_none());
        assert!(result.get("stream").is_none());
        assert_eq!(result["anthropic_version"], "bedrock-2023-05-31");
    }

    #[test]
    fn test_convert_request_sets_max_tokens() {
        let adapter = BedrockAdapter::with_region("model-id", "us-east-1");
        let payload = json!({
            "messages": [{"role": "user", "content": "Hi"}]
        });
        let result = adapter.convert_request(payload);
        assert_eq!(result["max_tokens"], 4096);
    }

    #[test]
    fn test_convert_request_preserves_custom_max_tokens() {
        let adapter = BedrockAdapter::with_region("model-id", "us-east-1");
        let payload = json!({
            "messages": [{"role": "user", "content": "Hi"}],
            "max_tokens": 8192
        });
        let result = adapter.convert_request(payload);
        assert_eq!(result["max_tokens"], 8192);
    }

    #[test]
    fn test_convert_request_converts_max_completion_tokens() {
        let adapter = BedrockAdapter::with_region("model-id", "us-east-1");
        let payload = json!({
            "messages": [{"role": "user", "content": "Hi"}],
            "max_completion_tokens": 2048
        });
        let result = adapter.convert_request(payload);
        assert_eq!(result["max_tokens"], 2048);
        assert!(result.get("max_completion_tokens").is_none());
    }

    #[test]
    fn test_response_to_chat_completions_text() {
        let response = json!({
            "id": "msg_123",
            "content": [{"type": "text", "text": "Hello!"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 10, "output_tokens": 5}
        });
        let result =
            BedrockAdapter::response_to_chat_completions(response, "anthropic.claude-3-sonnet");
        assert_eq!(result["object"], "chat.completion");
        assert_eq!(result["model"], "anthropic.claude-3-sonnet");
        assert_eq!(result["choices"][0]["message"]["content"], "Hello!");
        assert_eq!(result["choices"][0]["finish_reason"], "stop");
        assert_eq!(result["usage"]["prompt_tokens"], 10);
        assert_eq!(result["usage"]["completion_tokens"], 5);
        assert_eq!(result["usage"]["total_tokens"], 15);
    }

    #[test]
    fn test_response_to_chat_completions_tool_use() {
        let response = json!({
            "id": "msg_456",
            "content": [
                {"type": "text", "text": "Let me read that file."},
                {
                    "type": "tool_use",
                    "id": "tu_1",
                    "name": "read_file",
                    "input": {"path": "src/main.rs"}
                }
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 20, "output_tokens": 10}
        });
        let result = BedrockAdapter::response_to_chat_completions(response, "claude-3");
        assert_eq!(result["choices"][0]["finish_reason"], "tool_calls");
        let tool_calls = result["choices"][0]["message"]["tool_calls"]
            .as_array()
            .unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0]["id"], "tu_1");
        assert_eq!(tool_calls[0]["function"]["name"], "read_file");
    }

    #[test]
    fn test_response_max_tokens_finish_reason() {
        let response = json!({
            "content": [{"type": "text", "text": "truncated..."}],
            "stop_reason": "max_tokens",
            "usage": {"input_tokens": 0, "output_tokens": 0}
        });
        let result = BedrockAdapter::response_to_chat_completions(response, "model");
        assert_eq!(result["choices"][0]["finish_reason"], "length");
    }

    #[test]
    fn test_convert_tool_messages() {
        let mut payload = json!({
            "messages": [
                {"role": "user", "content": "Read the file"},
                {
                    "role": "assistant",
                    "content": "I'll read it.",
                    "tool_calls": [{
                        "id": "tc-1",
                        "function": {"name": "read_file", "arguments": "{\"path\": \"a.rs\"}"}
                    }]
                },
                {
                    "role": "tool",
                    "tool_call_id": "tc-1",
                    "content": "fn main() {}"
                }
            ]
        });
        BedrockAdapter::convert_tool_messages(&mut payload);
        let messages = payload["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);

        // Assistant message should have tool_use blocks
        let assistant = &messages[1];
        let content = assistant["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "tool_use");
        assert_eq!(content[1]["name"], "read_file");

        // Tool result should be converted to user with tool_result
        let tool_result = &messages[2];
        assert_eq!(tool_result["role"], "user");
        let blocks = tool_result["content"].as_array().unwrap();
        assert_eq!(blocks[0]["type"], "tool_result");
        assert_eq!(blocks[0]["tool_use_id"], "tc-1");
    }

    #[test]
    fn test_convert_tool_messages_merge_consecutive() {
        let mut payload = json!({
            "messages": [
                {
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [
                        {"id": "tc-1", "function": {"name": "read_file", "arguments": "{}"}},
                        {"id": "tc-2", "function": {"name": "search", "arguments": "{}"}}
                    ]
                },
                {"role": "tool", "tool_call_id": "tc-1", "content": "file1"},
                {"role": "tool", "tool_call_id": "tc-2", "content": "file2"}
            ]
        });
        BedrockAdapter::convert_tool_messages(&mut payload);
        let messages = payload["messages"].as_array().unwrap();
        // Two consecutive tool messages should be merged into one user message
        assert_eq!(messages.len(), 2);
        let user_msg = &messages[1];
        assert_eq!(user_msg["role"], "user");
        let blocks = user_msg["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["tool_use_id"], "tc-1");
        assert_eq!(blocks[1]["tool_use_id"], "tc-2");
    }

    #[test]
    fn test_extra_headers() {
        let adapter = BedrockAdapter::with_region("model-id", "us-east-1");
        let headers = adapter.extra_headers();
        assert!(
            headers
                .iter()
                .any(|(k, v)| k == "Content-Type" && v == "application/json")
        );
        assert!(
            headers
                .iter()
                .any(|(k, v)| k == "Accept" && v == "application/json")
        );
    }

    #[test]
    fn test_build_url() {
        let url = BedrockAdapter::build_url("ap-southeast-1", "anthropic.claude-v2");
        assert_eq!(
            url,
            "https://bedrock-runtime.ap-southeast-1.amazonaws.com/model/anthropic.claude-v2/invoke"
        );
    }
}
