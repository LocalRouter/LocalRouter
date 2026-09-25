//! Local Embedded providers: LocalRouter launches and supervises the inference engine
//! itself (llama.cpp, Laya, Kev, Von, Decider), with models managed in-app. Engines are
//! installed by the user through their package manager and found on PATH
//! (`lr_engines`).

pub mod decider;
pub mod kev;
pub mod laya;
pub mod von;

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use lr_engines::EngineError;
use lr_types::AppError;

pub use decider::{DeciderEmbeddedProvider, DeciderEmbeddedProviderFactory};
pub use kev::{KevEmbeddedProvider, KevEmbeddedProviderFactory};
pub use laya::{LayaEmbeddedProvider, LayaEmbeddedProviderFactory};
pub use von::{VonEmbeddedProvider, VonEmbeddedProviderFactory};

/// State of one model served by a Local Embedded provider.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EmbeddedModelState {
    /// Model id as clients use it (without the provider prefix).
    pub model: String,
    /// `running`, `exited` or `failed` for models with a process; models
    /// without one are omitted (unloaded).
    pub state: lr_engines::EngineState,
    pub port: Option<u16>,
    pub idle_secs: Option<u64>,
    pub last_error: Option<String>,
}

/// Controls a Local Embedded provider exposes to the app (Load/Unload buttons).
#[async_trait::async_trait]
pub trait EmbeddedControl: Send + Sync {
    /// Start the engine for `model` and wait until it is ready.
    async fn load(&self, model: &str) -> Result<(), AppError>;
    /// Stop the engine serving `model` (it starts again on the next request).
    async fn unload(&self, model: &str) -> Result<(), AppError>;
    /// Models that currently have (or recently had) an engine process.
    fn model_states(&self) -> Vec<EmbeddedModelState>;
}

/// Model states from the supervisor's processes whose key starts with
/// `prefix`; `model_of` maps a process key to the model id.
pub(crate) fn states_from_supervisor(
    supervisor: &lr_engines::Supervisor,
    prefix: &str,
    model_of: impl Fn(&str) -> Vec<String>,
) -> Vec<EmbeddedModelState> {
    supervisor
        .processes()
        .into_iter()
        .filter(|p| p.key.starts_with(prefix))
        .flat_map(|p| {
            model_of(&p.key)
                .into_iter()
                .map(|model| EmbeddedModelState {
                    model,
                    state: p.state,
                    port: p.port,
                    idle_secs: p.idle_secs,
                    last_error: p.last_error.clone(),
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

type TokenSource = Arc<dyn Fn() -> Option<String> + Send + Sync>;

static HF_TOKEN_SOURCE: RwLock<Option<TokenSource>> = RwLock::new(None);

/// Register where Local Embedded providers get the user's Hugging Face token (passed
/// to engines as `HF_TOKEN` for gated or private downloads).
pub fn set_hf_token_source(source: TokenSource) {
    *HF_TOKEN_SOURCE.write() = Some(source);
}

pub(crate) fn hf_token() -> Option<String> {
    HF_TOKEN_SOURCE.read().as_ref().and_then(|f| f())
}

/// Environment entries for the Hugging Face token, if the user signed in.
pub(crate) fn hf_env() -> Vec<(String, String)> {
    hf_token()
        .map(|t| vec![("HF_TOKEN".to_string(), t)])
        .unwrap_or_default()
}

/// Engine failures are reported as unreachable so auto-routing tries the
/// next model and the provider is marked unhealthy.
pub(crate) fn engine_error(provider: &str, e: EngineError) -> AppError {
    AppError::Provider(format!("Provider '{provider}' is unreachable: {e}"))
}

pub(crate) fn engine_missing(provider: &str, what: &str) -> AppError {
    AppError::Provider(format!(
        "Provider '{provider}' is unreachable: {what} was not found on PATH. Install it from the provider's Engine tab."
    ))
}

/// Parse a comma-separated list setting, keeping only allowed values in the
/// order the user gave. Unknown values are an error.
pub(crate) fn parse_list(
    config: &HashMap<String, String>,
    key: &str,
    allowed: &[&str],
    default: &[&str],
) -> Result<Vec<String>, AppError> {
    let raw = config.get(key).map(|s| s.trim()).unwrap_or_default();
    if raw.is_empty() {
        return Ok(default.iter().map(|s| s.to_string()).collect());
    }
    let mut out: Vec<String> = Vec::new();
    for item in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if !allowed.contains(&item) {
            return Err(AppError::Config(format!(
                "Unknown value '{item}' for {key}; expected one of: {}",
                allowed.join(", ")
            )));
        }
        if !out.iter().any(|x| x == item) {
            out.push(item.to_string());
        }
    }
    if out.is_empty() {
        return Err(AppError::Config(format!(
            "{key} must list at least one value"
        )));
    }
    Ok(out)
}

pub(crate) fn parse_minutes(
    config: &HashMap<String, String>,
    key: &str,
    default: u64,
) -> Result<Option<std::time::Duration>, AppError> {
    let minutes = match config.get(key).map(|s| s.trim()).filter(|s| !s.is_empty()) {
        Some(v) => v
            .parse::<u64>()
            .map_err(|_| AppError::Config(format!("{key} must be a whole number of minutes")))?,
        None => default,
    };
    Ok((minutes > 0).then(|| std::time::Duration::from_secs(minutes * 60)))
}

/// An HTTP client for one running System One engine, rebuilt when the engine
/// restarts on a new port or key.
#[derive(Default)]
pub(crate) struct SystemOneClientCache {
    inner: parking_lot::Mutex<HashMap<String, (u16, Arc<crate::systemone::SystemOneProvider>)>>,
}

impl SystemOneClientCache {
    pub(crate) fn get(
        &self,
        flavor: crate::systemone::SystemOneFlavor,
        handle: &lr_engines::EngineHandle,
    ) -> Result<Arc<crate::systemone::SystemOneProvider>, AppError> {
        let mut inner = self.inner.lock();
        if let Some((port, client)) = inner.get(&handle.key) {
            if *port == handle.port {
                return Ok(client.clone());
            }
        }
        let client = Arc::new(crate::systemone::SystemOneProvider::new(
            flavor,
            Some(handle.base_url()),
            Some(handle.api_key().to_string()),
        )?);
        inner.insert(handle.key.clone(), (handle.port, client.clone()));
        Ok(client)
    }
}

/// Look up an engine command off the async runtime (the first lookup may run
/// the user's login shell).
pub(crate) async fn resolve_engine(
    recipe: lr_engines::RecipeId,
    override_path: Option<std::path::PathBuf>,
) -> Option<lr_engines::EngineCommand> {
    tokio::task::spawn_blocking(move || lr_engines::resolve(recipe, override_path.as_deref()))
        .await
        .ok()
        .flatten()
}

/// The `lr-fake-engine` test binary from `lr-engines`, when it has been
/// built (it is during workspace test runs). Tests that need it skip
/// otherwise.
#[cfg(test)]
pub(crate) fn fake_engine_path() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.parent()?; // target/debug/deps -> target/debug
    let name = if cfg!(windows) {
        "lr-fake-engine.exe"
    } else {
        "lr-fake-engine"
    };
    let path = dir.join(name);
    path.is_file().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(k: &str, v: &str) -> HashMap<String, String> {
        [(k.to_string(), v.to_string())].into_iter().collect()
    }

    #[test]
    fn list_parsing() {
        let allowed = ["a", "b", "c"];
        assert_eq!(
            parse_list(&HashMap::new(), "x", &allowed, &["a"]).unwrap(),
            vec!["a"]
        );
        assert_eq!(
            parse_list(&cfg("x", " c, a ,c"), "x", &allowed, &["a"]).unwrap(),
            vec!["c", "a"]
        );
        assert!(parse_list(&cfg("x", "a,z"), "x", &allowed, &["a"]).is_err());
        assert!(parse_list(&cfg("x", " , "), "x", &allowed, &["a"]).is_err());
    }

    #[test]
    fn minutes_parsing() {
        assert_eq!(
            parse_minutes(&HashMap::new(), "m", 15).unwrap(),
            Some(std::time::Duration::from_secs(900))
        );
        assert_eq!(parse_minutes(&cfg("m", "0"), "m", 15).unwrap(), None);
        assert!(parse_minutes(&cfg("m", "x"), "m", 15).is_err());
    }

    #[test]
    fn engine_errors_classify_as_unreachable() {
        let e = engine_missing("laya", "laya-serve");
        assert!(e.to_string().contains("unreachable"));
    }
}
