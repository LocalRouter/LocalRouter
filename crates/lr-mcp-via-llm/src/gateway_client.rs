//! Convenience wrapper for MCP gateway JSON-RPC interactions
//!
//! Hides the JSON-RPC ceremony behind typed method calls.

use serde_json::{json, Value};

use crate::manager::McpViaLlmError;
use lr_config::Client;
use lr_mcp::protocol::{JsonRpcRequest, Root};
use lr_mcp::McpGateway;

/// Describes an MCP tool available via the gateway
#[derive(Debug, Clone)]
pub struct McpTool {
    /// Namespaced tool name (e.g. "filesystem__read_file")
    pub name: String,
    /// Tool description
    pub description: Option<String>,
    /// JSON Schema for the tool's input parameters
    pub input_schema: Value,
}

/// Describes an MCP resource available via the gateway
#[derive(Debug, Clone)]
pub struct McpResource {
    /// Namespaced resource name (e.g. "filesystem__config")
    pub name: String,
    /// Resource URI
    pub uri: String,
    /// Resource description
    pub description: Option<String>,
    /// MIME type
    pub mime_type: Option<String>,
}

/// Describes an MCP prompt available via the gateway
#[derive(Debug, Clone)]
pub struct McpPrompt {
    /// Namespaced prompt name (e.g. "github__pr_template")
    pub name: String,
    /// Prompt description
    pub description: Option<String>,
    /// Prompt arguments (empty = no-arg prompt)
    pub arguments: Vec<McpPromptArgument>,
}

/// A single argument for an MCP prompt
#[derive(Debug, Clone)]
pub struct McpPromptArgument {
    pub name: String,
    pub description: Option<String>,
    pub required: bool,
}

/// Wrapper around McpGateway for MCP via LLM operations
#[allow(dead_code)]
pub struct GatewayClient<'a> {
    gateway: &'a McpGateway,
    client_id: String,
    session_key: String,
    allowed_servers: Vec<String>,
    roots: Vec<Root>,
}

impl<'a> GatewayClient<'a> {
    /// Access the roots list (needed for spawning background tasks)
    pub fn roots(&self) -> &[Root] {
        &self.roots
    }

    /// Access the allowed servers list (needed for spawning background tasks)
    pub fn allowed_servers(&self) -> &[String] {
        &self.allowed_servers
    }

    pub fn new(
        gateway: &'a McpGateway,
        client: &Client,
        session_key: String,
        allowed_servers: Vec<String>,
    ) -> Self {
        let roots = client
            .roots
            .as_ref()
            .map(|rs| {
                rs.iter()
                    .filter(|r| r.enabled)
                    .map(|r| Root {
                        uri: r.uri.clone(),
                        name: r.name.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        Self {
            gateway,
            client_id: client.id.clone(),
            session_key,
            allowed_servers,
            roots,
        }
    }

    fn make_request(&self, method: &str, params: Option<Value>) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            method: method.to_string(),
            params,
        }
    }

    /// Initialize the gateway session (creates server connections)
    pub async fn initialize(&self) -> Result<(), McpViaLlmError> {
        let request = self.make_request(
            "initialize",
            Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "roots": { "listChanged": false },
                    "sampling": {}
                },
                "clientInfo": {
                    "name": "LocalRouter MCP-via-LLM",
                    "version": "1.0.0"
                }
            })),
        );

        let response = self
            .gateway
            .handle_request(
                &self.client_id,
                self.allowed_servers.clone(),
                self.roots.clone(),
                request,
            )
            .await
            .map_err(|e| McpViaLlmError::Gateway(format!("initialize failed: {}", e)))?;

        if let Some(error) = response.error {
            return Err(McpViaLlmError::Gateway(format!(
                "initialize error: {}",
                error.message
            )));
        }

        // Send initialized notification (required by MCP protocol)
        let notif_request = self.make_request("notifications/initialized", Some(json!({})));
        // Fire-and-forget: notifications don't return meaningful results
        let _ = self
            .gateway
            .handle_request(
                &self.client_id,
                self.allowed_servers.clone(),
                self.roots.clone(),
                notif_request,
            )
            .await;

        Ok(())
    }

    /// List all available MCP tools for this session
    pub async fn list_tools(&self) -> Result<Vec<McpTool>, McpViaLlmError> {
        let request = self.make_request("tools/list", Some(json!({})));

        let response = self
            .gateway
            .handle_request(
                &self.client_id,
                self.allowed_servers.clone(),
                self.roots.clone(),
                request,
            )
            .await
            .map_err(|e| McpViaLlmError::Gateway(format!("tools/list failed: {}", e)))?;

        if let Some(error) = response.error {
            return Err(McpViaLlmError::Gateway(format!(
                "tools/list error: {}",
                error.message
            )));
        }

        let result = response.result.unwrap_or(json!({"tools": []}));
        let tools_value = result.get("tools").cloned().unwrap_or_else(|| json!([]));

        let tools: Vec<McpTool> = tools_value
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| {
                        Some(McpTool {
                            name: t.get("name")?.as_str()?.to_string(),
                            description: t
                                .get("description")
                                .and_then(|d| d.as_str())
                                .map(|s| s.to_string()),
                            input_schema: t
                                .get("inputSchema")
                                .cloned()
                                .unwrap_or_else(|| json!({"type": "object"})),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(tools)
    }

    /// Execute an MCP tool call and return the result content
    pub async fn call_tool(
        &self,
        tool_name: &str,
        arguments: Value,
    ) -> Result<Value, McpViaLlmError> {
        let request = self.make_request(
            "tools/call",
            Some(json!({
                "name": tool_name,
                "arguments": arguments
            })),
        );

        let response = self
            .gateway
            .handle_request(
                &self.client_id,
                self.allowed_servers.clone(),
                self.roots.clone(),
                request,
            )
            .await
            .map_err(|e| {
                McpViaLlmError::ToolExecution(format!("tools/call '{}' failed: {}", tool_name, e))
            })?;

        if let Some(error) = response.error {
            return Err(McpViaLlmError::ToolExecution(format!(
                "tools/call '{}' error: {}",
                tool_name, error.message
            )));
        }

        let result = response.result.unwrap_or(json!({}));

        // Extract text content from MCP tool result
        // MCP returns: { content: [{ type: "text", text: "..." }], isError: false }
        if let Some(content) = result.get("content") {
            if let Some(arr) = content.as_array() {
                let texts: Vec<String> = arr
                    .iter()
                    .filter_map(|c| {
                        if c.get("type").and_then(|t| t.as_str()) == Some("text") {
                            c.get("text")
                                .and_then(|t| t.as_str())
                                .map(|s| s.to_string())
                        } else {
                            None
                        }
                    })
                    .collect();

                if texts.len() == 1 {
                    return Ok(Value::String(texts.into_iter().next().unwrap()));
                } else if !texts.is_empty() {
                    return Ok(Value::String(texts.join("\n")));
                }
            }
        }

        // Fallback: return the raw result
        Ok(result)
    }

    /// List all available MCP resources
    pub async fn list_resources(&self) -> Result<Vec<McpResource>, McpViaLlmError> {
        let request = self.make_request("resources/list", Some(json!({})));

        let response = self
            .gateway
            .handle_request(
                &self.client_id,
                self.allowed_servers.clone(),
                self.roots.clone(),
                request,
            )
            .await
            .map_err(|e| McpViaLlmError::Gateway(format!("resources/list failed: {}", e)))?;

        if let Some(error) = response.error {
            return Err(McpViaLlmError::Gateway(format!(
                "resources/list error: {}",
                error.message
            )));
        }

        let result = response.result.unwrap_or(json!({"resources": []}));
        let resources_value = result
            .get("resources")
            .cloned()
            .unwrap_or_else(|| json!([]));

        let resources: Vec<McpResource> = resources_value
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|r| {
                        Some(McpResource {
                            name: r.get("name")?.as_str()?.to_string(),
                            uri: r.get("uri")?.as_str()?.to_string(),
                            description: r
                                .get("description")
                                .and_then(|d| d.as_str())
                                .map(|s| s.to_string()),
                            mime_type: r
                                .get("mimeType")
                                .and_then(|m| m.as_str())
                                .map(|s| s.to_string()),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(resources)
    }

    /// Read an MCP resource by URI
    pub async fn read_resource(&self, uri: &str) -> Result<String, McpViaLlmError> {
        let request = self.make_request(
            "resources/read",
            Some(json!({
                "uri": uri
            })),
        );

        let response = self
            .gateway
            .handle_request(
                &self.client_id,
                self.allowed_servers.clone(),
                self.roots.clone(),
                request,
            )
            .await
            .map_err(|e| {
                McpViaLlmError::ToolExecution(format!("resources/read '{}' failed: {}", uri, e))
            })?;

        if let Some(error) = response.error {
            return Err(McpViaLlmError::ToolExecution(format!(
                "resources/read '{}' error: {}",
                uri, error.message
            )));
        }

        let result = response.result.unwrap_or(json!({}));

        // Extract text content: { contents: [{ uri, text, mimeType }] }
        if let Some(contents) = result.get("contents").and_then(|c| c.as_array()) {
            let texts: Vec<String> = contents
                .iter()
                .filter_map(|c| {
                    c.get("text")
                        .and_then(|t| t.as_str())
                        .map(|s| s.to_string())
                })
                .collect();
            if !texts.is_empty() {
                return Ok(texts.join("\n"));
            }
        }

        Ok(result.to_string())
    }

    /// List all available MCP prompts
    pub async fn list_prompts(&self) -> Result<Vec<McpPrompt>, McpViaLlmError> {
        let request = self.make_request("prompts/list", Some(json!({})));

        let response = self
            .gateway
            .handle_request(
                &self.client_id,
                self.allowed_servers.clone(),
                self.roots.clone(),
                request,
            )
            .await
            .map_err(|e| McpViaLlmError::Gateway(format!("prompts/list failed: {}", e)))?;

        if let Some(error) = response.error {
            return Err(McpViaLlmError::Gateway(format!(
                "prompts/list error: {}",
                error.message
            )));
        }

        let result = response.result.unwrap_or(json!({"prompts": []}));
        let prompts_value = result.get("prompts").cloned().unwrap_or_else(|| json!([]));

        let prompts: Vec<McpPrompt> = prompts_value
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| {
                        let arguments = p
                            .get("arguments")
                            .and_then(|a| a.as_array())
                            .map(|args| {
                                args.iter()
                                    .filter_map(|arg| {
                                        Some(McpPromptArgument {
                                            name: arg.get("name")?.as_str()?.to_string(),
                                            description: arg
                                                .get("description")
                                                .and_then(|d| d.as_str())
                                                .map(|s| s.to_string()),
                                            required: arg
                                                .get("required")
                                                .and_then(|r| r.as_bool())
                                                .unwrap_or(false),
                                        })
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();

                        Some(McpPrompt {
                            name: p.get("name")?.as_str()?.to_string(),
                            description: p
                                .get("description")
                                .and_then(|d| d.as_str())
                                .map(|s| s.to_string()),
                            arguments,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(prompts)
    }

    /// Get a prompt with arguments, returning the resolved messages
    pub async fn get_prompt(
        &self,
        prompt_name: &str,
        arguments: Value,
    ) -> Result<Vec<Value>, McpViaLlmError> {
        let request = self.make_request(
            "prompts/get",
            Some(json!({
                "name": prompt_name,
                "arguments": arguments
            })),
        );

        let response = self
            .gateway
            .handle_request(
                &self.client_id,
                self.allowed_servers.clone(),
                self.roots.clone(),
                request,
            )
            .await
            .map_err(|e| {
                McpViaLlmError::ToolExecution(format!(
                    "prompts/get '{}' failed: {}",
                    prompt_name, e
                ))
            })?;

        if let Some(error) = response.error {
            return Err(McpViaLlmError::ToolExecution(format!(
                "prompts/get '{}' error: {}",
                prompt_name, error.message
            )));
        }

        let result = response.result.unwrap_or(json!({}));

        // Extract messages: { messages: [{ role, content: { type, text } }] }
        let messages = result
            .get("messages")
            .and_then(|m| m.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(messages)
    }
}
