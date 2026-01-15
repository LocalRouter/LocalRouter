//! Graph visualization data structures and logic
//!
//! Builds graph data for visualizing the relationships between
//! providers, models, and API keys.

use crate::config::{ActiveRoutingStrategy, ApiKeyConfig};
use crate::providers::registry::ProviderInstanceInfo;
use crate::providers::{Capability, ModelInfo, ProviderHealth};
use crate::ui::commands::ApiKeyInfo;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Graph data structure for visualization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualizationGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

/// A node in the visualization graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: NodeType,
    pub label: String,
    pub data: NodeData,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
}

/// Node position (optional, for custom layouts)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

/// Type of node in the graph
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NodeType {
    Provider,
    Model,
    ApiKey,
    AddProvider,
    AddApiKey,
}

/// Node-specific data
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "nodeType")]
pub enum NodeData {
    Provider {
        instance_name: String,
        provider_type: String,
        health: ProviderHealth,
        enabled: bool,
    },
    Model {
        model_id: String,
        provider_instance: String,
        capabilities: Vec<String>,
        context_window: u32,
        supports_streaming: bool,
    },
    ApiKey {
        key_id: String,
        key_name: String,
        enabled: bool,
        created_at: String,
        routing_strategy: Option<String>,
    },
    AddNode,
}

/// An edge in the visualization graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    #[serde(rename = "type")]
    pub edge_type: EdgeType,
}

/// Type of edge in the graph
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EdgeType {
    ProviderToModel,
    ApiKeyToModel,
}

/// Build the visualization graph from current system state
pub fn build_graph(
    providers: Vec<ProviderInstanceInfo>,
    models: Vec<ModelInfo>,
    api_keys: Vec<ApiKeyInfo>,
    api_key_configs: Vec<ApiKeyConfig>,
    health_statuses: HashMap<String, ProviderHealth>,
) -> VisualizationGraph {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    // 1. Create provider nodes
    for provider in &providers {
        let health = health_statuses
            .get(&provider.instance_name)
            .cloned()
            .unwrap_or_else(|| ProviderHealth {
                status: crate::providers::HealthStatus::Unhealthy,
                latency_ms: None,
                last_checked: Utc::now(),
                error_message: Some("No health data available".to_string()),
            });

        nodes.push(GraphNode {
            id: format!("provider:{}", provider.instance_name),
            node_type: NodeType::Provider,
            label: provider.instance_name.clone(),
            data: NodeData::Provider {
                instance_name: provider.instance_name.clone(),
                provider_type: provider.provider_type.clone(),
                health,
                enabled: provider.enabled,
            },
            position: None,
        });
    }

    // 2. Create model nodes and provider->model edges
    for model in &models {
        let capabilities: Vec<String> = model
            .capabilities
            .iter()
            .map(|c| capability_to_string(c))
            .collect();

        let model_node_id = format!("model:{}:{}", model.provider, model.id);

        nodes.push(GraphNode {
            id: model_node_id.clone(),
            node_type: NodeType::Model,
            label: model.name.clone(),
            data: NodeData::Model {
                model_id: model.id.clone(),
                provider_instance: model.provider.clone(),
                capabilities,
                context_window: model.context_window,
                supports_streaming: model.supports_streaming,
            },
            position: None,
        });

        // Create edge from provider to model
        let provider_node_id = format!("provider:{}", model.provider);
        edges.push(GraphEdge {
            id: format!("edge:{}:{}", provider_node_id, model_node_id),
            source: provider_node_id,
            target: model_node_id,
            edge_type: EdgeType::ProviderToModel,
        });
    }

    // 3. Create API key nodes and apikey->model edges
    for api_key in &api_keys {
        // Find the full config to get routing info
        let config = api_key_configs
            .iter()
            .find(|c| c.id == api_key.id);

        let routing_strategy = config
            .and_then(|c| c.get_routing_config())
            .as_ref()
            .map(|rc| routing_strategy_to_string(&rc.active_strategy));

        let api_key_node_id = format!("apikey:{}", api_key.id);

        nodes.push(GraphNode {
            id: api_key_node_id.clone(),
            node_type: NodeType::ApiKey,
            label: api_key.name.clone(),
            data: NodeData::ApiKey {
                key_id: api_key.id.clone(),
                key_name: api_key.name.clone(),
                enabled: api_key.enabled,
                created_at: api_key.created_at.clone(),
                routing_strategy,
            },
            position: None,
        });

        // Create edges from API key to models based on routing config
        if let Some(config) = config {
            let allowed_models = get_allowed_models_for_key(config, &models);
            for (provider, model_id) in allowed_models {
                let model_node_id = format!("model:{}:{}", provider, model_id);
                edges.push(GraphEdge {
                    id: format!("edge:{}:{}", api_key_node_id, model_node_id),
                    source: api_key_node_id.clone(),
                    target: model_node_id,
                    edge_type: EdgeType::ApiKeyToModel,
                });
            }
        }
    }

    // 4. Add special "+" nodes
    nodes.push(GraphNode {
        id: "add:provider".to_string(),
        node_type: NodeType::AddProvider,
        label: "+ Provider".to_string(),
        data: NodeData::AddNode,
        position: None,
    });

    nodes.push(GraphNode {
        id: "add:apikey".to_string(),
        node_type: NodeType::AddApiKey,
        label: "+ API Key".to_string(),
        data: NodeData::AddNode,
        position: None,
    });

    VisualizationGraph { nodes, edges }
}

/// Get the list of models allowed for an API key
fn get_allowed_models_for_key(
    config: &ApiKeyConfig,
    all_models: &[ModelInfo],
) -> Vec<(String, String)> {
    let mut allowed = Vec::new();

    if let Some(routing_config) = config.get_routing_config() {
        match routing_config.active_strategy {
            ActiveRoutingStrategy::AvailableModels => {
                // Include all models from providers with all_provider_models
                for provider_name in &routing_config.available_models.all_provider_models {
                    for model in all_models {
                        if &model.provider == provider_name {
                            allowed.push((model.provider.clone(), model.id.clone()));
                        }
                    }
                }
                // Include individual models
                allowed.extend(routing_config.available_models.individual_models.clone());
            }
            ActiveRoutingStrategy::ForceModel => {
                // Only the forced model
                if let Some((provider, model)) = routing_config.forced_model {
                    allowed.push((provider, model));
                }
            }
            ActiveRoutingStrategy::PrioritizedList => {
                // All models in the prioritized list
                allowed.extend(routing_config.prioritized_models.clone());
            }
        }
    } else if let Some(ref model_selection) = config.model_selection {
        // Legacy model selection support
        use crate::config::ModelSelection;
        match model_selection {
            ModelSelection::All => {
                // All models
                for model in all_models {
                    allowed.push((model.provider.clone(), model.id.clone()));
                }
            }
            ModelSelection::Custom {
                all_provider_models,
                individual_models,
            } => {
                // All models from specified providers
                for provider_name in all_provider_models {
                    for model in all_models {
                        if &model.provider == provider_name {
                            allowed.push((model.provider.clone(), model.id.clone()));
                        }
                    }
                }
                // Plus individual models
                allowed.extend(individual_models.clone());
            }
            ModelSelection::DirectModel { provider, model } => {
                allowed.push((provider.clone(), model.clone()));
            }
            ModelSelection::Router { .. } => {
                // Router-based selection - not directly visualizable
                // Could potentially show all available models, but for now skip
            }
        }
    }

    allowed
}

/// Convert capability enum to string
fn capability_to_string(cap: &Capability) -> String {
    match cap {
        Capability::Chat => "Chat".to_string(),
        Capability::Completion => "Completion".to_string(),
        Capability::Embedding => "Embedding".to_string(),
        Capability::Vision => "Vision".to_string(),
        Capability::FunctionCalling => "FunctionCalling".to_string(),
    }
}

/// Convert routing strategy enum to string
fn routing_strategy_to_string(strategy: &ActiveRoutingStrategy) -> String {
    match strategy {
        ActiveRoutingStrategy::AvailableModels => "Available Models".to_string(),
        ActiveRoutingStrategy::ForceModel => "Force Model".to_string(),
        ActiveRoutingStrategy::PrioritizedList => "Prioritized List".to_string(),
    }
}
