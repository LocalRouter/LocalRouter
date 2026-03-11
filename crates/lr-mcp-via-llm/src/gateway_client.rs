//! Convenience wrapper for MCP gateway JSON-RPC interactions
//!
//! Hides the JSON-RPC ceremony behind typed method calls.

use serde_json::{json, Value};

use crate::manager::McpViaLlmError;
use lr_config::Client;
use lr_mcp::protocol::{JsonRpcRequest, JsonRpcResponse, Root};
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
    // Client permissions for virtual server access
    mcp_permissions: lr_config::McpPermissions,
    skills_permissions: lr_config::SkillsPermissions,
    client_name: String,
    marketplace_permission: lr_config::PermissionState,
    coding_agent_permission: lr_config::PermissionState,
    coding_agent_type: Option<lr_config::CodingAgentType>,
    context_management_overrides: Option<lr_config::ContextManagementOverrides>,
    mcp_sampling_permission: lr_config::PermissionState,
    mcp_elicitation_permission: lr_config::PermissionState,
}

/// Build the MCP initialize capabilities JSON based on permission settings.
pub(crate) fn build_init_capabilities(
    sampling_permission: &lr_config::PermissionState,
    elicitation_permission: &lr_config::PermissionState,
) -> Value {
    let mut capabilities = json!({
        "roots": { "listChanged": false }
    });
    if !matches!(sampling_permission, lr_config::PermissionState::Off) {
        capabilities["sampling"] = json!({});
    }
    if !matches!(elicitation_permission, lr_config::PermissionState::Off) {
        capabilities["elicitation"] = json!({});
    }
    capabilities
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
            mcp_permissions: client.mcp_permissions.clone(),
            skills_permissions: client.skills_permissions.clone(),
            client_name: client.name.clone(),
            marketplace_permission: client.marketplace_permission.clone(),
            coding_agent_permission: client.coding_agent_permission.clone(),
            coding_agent_type: client.coding_agent_type,
            context_management_overrides: Some(lr_config::ContextManagementOverrides {
                context_management_enabled: client.context_management_enabled,
                indexing_tools_enabled: client.indexing_tools_enabled,
                catalog_compression_enabled: client.catalog_compression_enabled,
            }),
            mcp_sampling_permission: client.mcp_sampling_permission.clone(),
            mcp_elicitation_permission: client.mcp_elicitation_permission.clone(),
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

    /// Send a request through the gateway with full virtual server permissions
    async fn send_request(&self, request: JsonRpcRequest) -> lr_types::AppResult<JsonRpcResponse> {
        self.gateway
            .handle_request_with_skills(
                &self.client_id,
                Some(&self.session_key),
                self.allowed_servers.clone(),
                self.roots.clone(),
                self.mcp_permissions.clone(),
                self.skills_permissions.clone(),
                self.client_name.clone(),
                self.marketplace_permission.clone(),
                self.coding_agent_permission.clone(),
                self.coding_agent_type,
                self.context_management_overrides.clone(),
                self.mcp_sampling_permission.clone(),
                self.mcp_elicitation_permission.clone(),
                request,
            )
            .await
    }

    /// Initialize the gateway session (creates server connections).
    /// Returns the unified gateway instructions if any MCP servers provided them.
    pub async fn initialize(&self) -> Result<Option<String>, McpViaLlmError> {
        let capabilities = build_init_capabilities(
            &self.mcp_sampling_permission,
            &self.mcp_elicitation_permission,
        );

        let request = self.make_request(
            "initialize",
            Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": capabilities,
                "clientInfo": {
                    "name": "LocalRouter MCP-via-LLM",
                    "version": "1.0.0"
                }
            })),
        );

        let response = self
            .send_request(request)
            .await
            .map_err(|e| McpViaLlmError::Gateway(format!("initialize failed: {}", e)))?;

        if let Some(error) = response.error {
            return Err(McpViaLlmError::Gateway(format!(
                "initialize error: {}",
                error.message
            )));
        }

        // Extract unified gateway instructions from the response
        let instructions = response
            .result
            .as_ref()
            .and_then(|r| r.get("instructions"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Send initialized notification (required by MCP protocol)
        let notif_request = self.make_request("notifications/initialized", Some(json!({})));
        // Fire-and-forget: notifications don't return meaningful results
        let _ = self.send_request(notif_request).await;

        Ok(instructions)
    }

    /// List all available MCP tools for this session
    pub async fn list_tools(&self) -> Result<Vec<McpTool>, McpViaLlmError> {
        let request = self.make_request("tools/list", Some(json!({})));

        let response = self
            .send_request(request)
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

        let response = self.send_request(request).await.map_err(|e| {
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

        // Fallback: return the raw result as JSON string
        tracing::debug!(
            "MCP via LLM: tool '{}' returned non-text content, using raw JSON",
            tool_name
        );
        Ok(result)
    }

    /// Read an MCP resource by namespaced name.
    ///
    /// Uses the gateway's name-based routing (which looks up server_id and
    /// original name from the resource mapping).
    pub async fn read_resource(&self, name: &str) -> Result<String, McpViaLlmError> {
        let request = self.make_request(
            "resources/read",
            Some(json!({
                "name": name
            })),
        );

        let response = self.send_request(request).await.map_err(|e| {
            McpViaLlmError::ToolExecution(format!("resources/read '{}' failed: {}", name, e))
        })?;

        if let Some(error) = response.error {
            return Err(McpViaLlmError::ToolExecution(format!(
                "resources/read '{}' error: {}",
                name, error.message
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

    /// Read a skill file via the gateway's skill virtual server.
    ///
    /// The gateway routes this to the SkillsVirtualServer which reads the
    /// file from disk after permission checks.
    pub async fn read_skill_file(
        &self,
        skill_name: &str,
        subpath: &str,
    ) -> Result<String, McpViaLlmError> {
        // Use the skill_read tool with a special "__file" action
        // Actually, we call the skill file reader directly via a tools/call
        // that the skills virtual server handles
        let request = self.make_request(
            "tools/call",
            Some(json!({
                "name": "skill_read_file",
                "arguments": {
                    "skill": skill_name,
                    "path": subpath
                }
            })),
        );

        let response = self.send_request(request).await.map_err(|e| {
            McpViaLlmError::ToolExecution(format!(
                "skill file read '{}/{}' failed: {}",
                skill_name, subpath, e
            ))
        })?;

        if let Some(error) = response.error {
            return Err(McpViaLlmError::ToolExecution(format!(
                "skill file read '{}/{}' error: {}",
                skill_name, subpath, error.message
            )));
        }

        let result = response.result.unwrap_or(json!({}));

        // Extract text from tool result content
        if let Some(content) = result.get("content").and_then(|c| c.as_array()) {
            let texts: Vec<String> = content
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
            .send_request(request)
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

        let response = self.send_request(request).await.map_err(|e| {
            McpViaLlmError::ToolExecution(format!("prompts/get '{}' failed: {}", prompt_name, e))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sampling_off_no_capability() {
        let caps = build_init_capabilities(
            &lr_config::PermissionState::Off,
            &lr_config::PermissionState::Ask,
        );
        assert!(caps.get("roots").is_some());
        assert!(
            caps.get("sampling").is_none(),
            "sampling should be absent when Off"
        );
        assert!(
            caps.get("elicitation").is_some(),
            "elicitation should be present when Ask"
        );
    }

    #[test]
    fn test_sampling_ask_has_capability() {
        let caps = build_init_capabilities(
            &lr_config::PermissionState::Ask,
            &lr_config::PermissionState::Off,
        );
        assert!(
            caps.get("sampling").is_some(),
            "sampling should be present when Ask"
        );
        assert!(
            caps.get("elicitation").is_none(),
            "elicitation should be absent when Off"
        );
    }

    #[test]
    fn test_sampling_allow_has_capability() {
        let caps = build_init_capabilities(
            &lr_config::PermissionState::Allow,
            &lr_config::PermissionState::Allow,
        );
        assert!(
            caps.get("sampling").is_some(),
            "sampling should be present when Allow"
        );
        assert!(
            caps.get("elicitation").is_some(),
            "elicitation should be present when Allow"
        );
    }

    #[test]
    fn test_elicitation_off_no_capability() {
        let caps = build_init_capabilities(
            &lr_config::PermissionState::Allow,
            &lr_config::PermissionState::Off,
        );
        assert!(
            caps.get("elicitation").is_none(),
            "elicitation should be absent when Off"
        );
    }

    #[test]
    fn test_both_off_only_roots() {
        let caps = build_init_capabilities(
            &lr_config::PermissionState::Off,
            &lr_config::PermissionState::Off,
        );
        assert!(caps.get("roots").is_some());
        assert!(caps.get("sampling").is_none());
        assert!(caps.get("elicitation").is_none());
        assert_eq!(caps.as_object().unwrap().len(), 1);
    }
}
