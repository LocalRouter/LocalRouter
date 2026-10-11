//! FastFlowLM: which models are installed.
//!
//! FastFlowLM's server (`flm serve`, OpenAI-compatible on port 52625) lists
//! every model it supports in `GET /v1/models` and `/api/tags`, installed or
//! not, and requesting one that is missing starts a multi-GB download that
//! blocks the request. So the provider lists the installed models instead,
//! found the way `flm` itself finds them (upstream `src/src/main.cpp`,
//! `src/common/utils.cpp`, `src/include/model_list.hpp`):
//!
//! 1. `flm list --filter installed --json` prints
//!    `{"models": [{"name": "<family>:<size>", ..., "installed": true}]}`,
//!    each entry being the model's catalog record.
//! 2. Without the CLI: the model folder is `FLM_MODEL_PATH`, else
//!    `%USERPROFILE%\.flm` on Windows and `$HOME/.config/flm` elsewhere; a
//!    model is installed when `<folder>/<catalog model_path>/<entry name>/
//!    config.json` exists, with entries from the catalog `model_list.json`
//!    (`FLM_CONFIG_PATH`, next to `flm`, `<flm dir>/../share/flm/`, or the
//!    installers' locations).
//!
//! Both only describe this machine, so they are used only when the base URL
//! is a loopback address. When neither answers, the caller falls back to the
//! server's full listing with new models hidden.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::{Capability, ModelInfo};

/// Default FastFlowLM server port (`FLM_SERVE_PORT` overrides it upstream).
pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:52625/v1";

/// How long `flm list` may take before it is abandoned.
const CLI_TIMEOUT: Duration = Duration::from_secs(15);

/// Finds the models FastFlowLM has installed on this machine.
#[derive(Debug, Clone)]
pub struct InstalledModels {
    cli_timeout: Duration,
    /// Look at this machine at all (off in tests that must not depend on
    /// whether FastFlowLM is installed where they run)
    inspect_machine: bool,
}

impl Default for InstalledModels {
    fn default() -> Self {
        Self {
            cli_timeout: CLI_TIMEOUT,
            inspect_machine: true,
        }
    }
}

impl InstalledModels {
    pub fn new() -> Self {
        Self::default()
    }

    /// A detector that never finds anything (tests).
    #[cfg(test)]
    pub(crate) fn unavailable() -> Self {
        Self {
            inspect_machine: false,
            ..Self::default()
        }
    }

    /// The installed models, or `None` when they cannot be determined here
    /// (remote server, no CLI and no readable model folder).
    pub async fn detect(&self, instance: &str, base_url: &str) -> Option<Vec<ModelInfo>> {
        if !self.inspect_machine || !is_loopback_url(base_url) {
            return None;
        }
        let flm = tokio::task::spawn_blocking(|| lr_utils::binary::find_binary("flm"))
            .await
            .ok()
            .flatten();
        if let Some(flm) = &flm {
            match run_cli(flm, self.cli_timeout).await {
                Ok(entries) => return Some(models_from_entries(&entries, instance)),
                Err(e) => {
                    tracing::debug!("`flm list` did not answer ({e}); scanning the model folder")
                }
            }
        }
        let locations = Locations::from_env(flm.as_deref());
        let entries = tokio::task::spawn_blocking(move || scan_installed(&locations))
            .await
            .ok()
            .flatten()?;
        Some(models_from_entries(&entries, instance))
    }
}

/// Whether a base URL points at this machine.
pub fn is_loopback_url(base_url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(base_url.trim()) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// A catalog entry: its tag (`family:size`) and record.
type Entry = (String, Value);

async fn run_cli(flm: &Path, timeout: Duration) -> Result<Vec<Entry>, String> {
    let mut cmd = tokio::process::Command::new(flm);
    cmd.args(["list", "--filter", "installed", "--json"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let output = tokio::time::timeout(timeout, cmd.output())
        .await
        .map_err(|_| format!("timed out after {}s", timeout.as_secs()))?
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("exited with {}", output.status));
    }
    parse_cli_list(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| "unrecognized output".to_string())
}

/// Parse `flm list --filter installed --json`. Status lines may precede the
/// JSON document, so it is read from the first line that opens an object.
pub(crate) fn parse_cli_list(stdout: &str) -> Option<Vec<Entry>> {
    let mut offset = 0;
    for line in stdout.split_inclusive('\n') {
        if line.trim_start().starts_with('{') {
            let mut stream =
                serde_json::Deserializer::from_str(&stdout[offset..]).into_iter::<Value>();
            if let Some(Ok(doc)) = stream.next() {
                let models = doc.get("models")?.as_array()?;
                return Some(
                    models
                        .iter()
                        .filter(|m| m.get("installed").and_then(Value::as_bool) != Some(false))
                        .filter_map(|m| {
                            let tag = m.get("name")?.as_str()?.to_string();
                            Some((tag, m.clone()))
                        })
                        .collect(),
                );
            }
        }
        offset += line.len();
    }
    None
}

/// Where FastFlowLM keeps its models and its catalog.
#[derive(Debug, Clone, Default)]
pub(crate) struct Locations {
    /// The model folder (`FLM_MODEL_PATH` or the platform default)
    pub models_dir: Option<PathBuf>,
    /// Candidate `model_list.json` paths, in lookup order
    pub catalogs: Vec<PathBuf>,
}

impl Locations {
    fn from_env(flm: Option<&Path>) -> Self {
        let env = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let models_dir = env("FLM_MODEL_PATH").or_else(|| {
            if cfg!(windows) {
                env("USERPROFILE").map(|h| h.join(".flm"))
            } else {
                env("HOME").map(|h| h.join(".config").join("flm"))
            }
        });
        let mut catalogs = Vec::new();
        catalogs.extend(env("FLM_CONFIG_PATH"));
        // `flm` may be a symlink (e.g. /usr/bin/flm -> /opt/fastflowlm/bin/flm).
        if let Some(flm) = flm {
            let resolved = std::fs::canonicalize(flm).unwrap_or_else(|_| flm.to_path_buf());
            for bin in [Some(resolved), Some(flm.to_path_buf())]
                .into_iter()
                .flatten()
            {
                if let Some(dir) = bin.parent() {
                    catalogs.push(dir.join("model_list.json"));
                    catalogs.push(dir.join("../share/flm/model_list.json"));
                }
            }
        }
        if cfg!(windows) {
            for base in ["ProgramFiles", "LOCALAPPDATA"] {
                if let Some(dir) = env(base) {
                    catalogs.push(dir.join("flm").join("model_list.json"));
                }
            }
        } else {
            for path in [
                "/opt/fastflowlm/share/flm/model_list.json",
                "/usr/share/flm/model_list.json",
                "/usr/local/share/flm/model_list.json",
            ] {
                catalogs.push(PathBuf::from(path));
            }
        }
        Self {
            models_dir,
            catalogs,
        }
    }
}

/// The catalog entries whose model folder holds a `config.json`, or `None`
/// when the model folder or the catalog cannot be read.
pub(crate) fn scan_installed(locations: &Locations) -> Option<Vec<Entry>> {
    let models_dir = locations.models_dir.as_ref().filter(|d| d.is_dir())?;
    let catalog = locations.catalogs.iter().find_map(|path| {
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str::<Value>(&text).ok()
    })?;
    let root = models_dir.join(
        catalog
            .get("model_path")
            .and_then(Value::as_str)
            .unwrap_or("models"),
    );
    let mut found = Vec::new();
    for (family, sizes) in catalog.get("models")?.as_object()? {
        let Some(sizes) = sizes.as_object() else {
            continue;
        };
        for (size, record) in sizes {
            let Some(folder) = record.get("name").and_then(Value::as_str) else {
                continue;
            };
            if root.join(folder).join("config.json").is_file() {
                found.push((format!("{family}:{size}"), record.clone()));
            }
        }
    }
    Some(found)
}

fn models_from_entries(entries: &[Entry], instance: &str) -> Vec<ModelInfo> {
    entries
        .iter()
        .map(|(tag, record)| model_from_record(tag, record, instance))
        .collect()
}

/// A model described from its FastFlowLM catalog record. Capabilities come
/// from the record's declared `label` list and `vlm` flag.
pub(crate) fn model_from_record(tag: &str, record: &Value, instance: &str) -> ModelInfo {
    let labels: Vec<&str> = record
        .get("label")
        .and_then(Value::as_array)
        .map(|l| l.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let has = |label: &str| labels.contains(&label);
    let capabilities = if has("embeddings") {
        vec![Capability::Embedding]
    } else if has("transcription") {
        vec![Capability::Audio]
    } else {
        let mut caps = vec![Capability::Chat, Capability::Completion];
        if has("vision") || record.get("vlm").and_then(Value::as_bool) == Some(true) {
            caps.push(Capability::Vision);
        }
        if has("tool-calling") {
            caps.push(Capability::FunctionCalling);
        }
        caps
    };
    let context_window = record
        .get("default_context_length")
        .and_then(Value::as_u64)
        .and_then(|c| u32::try_from(c).ok())
        .filter(|c| *c > 0)
        .unwrap_or(crate::model_settings::DEFAULT_CONTEXT_WINDOW);
    let parameter_count = record
        .pointer("/details/parameter_size")
        .and_then(Value::as_str)
        .and_then(parse_parameter_size);
    ModelInfo {
        id: tag.to_string(),
        name: tag.to_string(),
        provider: instance.to_string(),
        parameter_count,
        context_window,
        supports_streaming: true,
        capabilities,
        detailed_capabilities: None,
    }
}

/// `"1.8B"` → 1.8e9, `"300M"` → 3e8.
fn parse_parameter_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num, mult) = match s.chars().last()? {
        'B' | 'b' => (&s[..s.len() - 1], 1e9),
        'M' | 'm' => (&s[..s.len() - 1], 1e6),
        _ => return None,
    };
    let n: f64 = num.trim().parse().ok()?;
    (n > 0.0).then(|| (n * mult).round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn loopback_urls() {
        for url in [
            "http://127.0.0.1:52625/v1",
            "http://localhost:52625/v1",
            "http://LOCALHOST/v1",
            "http://[::1]:52625/v1",
            "  http://127.0.0.2:1/  ",
        ] {
            assert!(is_loopback_url(url), "{url}");
        }
        for url in [
            "http://192.168.1.5:52625/v1",
            "https://example.com/v1",
            "http://localhost.example.com/v1",
            "not a url",
        ] {
            assert!(!is_loopback_url(url), "{url}");
        }
    }

    #[test]
    fn cli_output_is_parsed_after_status_lines() {
        let out = "[FLM]  corelib unavailable: no device\n{\n    \"models\": [\n        {\"name\": \"llama3.2:1b\", \"installed\": true, \"default_context_length\": 131072},\n        {\"name\": \"qwen3:4b\", \"installed\": false}\n    ]\n}\n";
        let entries = parse_cli_list(out).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "llama3.2:1b");
        assert_eq!(parse_cli_list("{\"models\": []}\n").unwrap().len(), 0);
        assert!(parse_cli_list("Models:\n  - llama3.2:1b\n").is_none());
        assert!(parse_cli_list("{\"other\": 1}").is_none());
    }

    #[test]
    fn records_map_to_declared_capabilities() {
        let chat = model_from_record(
            "qwen3vl-it:4b",
            &json!({"label": ["vision", "tool-calling"], "vlm": true,
                    "default_context_length": 32768,
                    "details": {"parameter_size": "4B"}}),
            "flm",
        );
        assert_eq!(
            chat.capabilities,
            vec![
                Capability::Chat,
                Capability::Completion,
                Capability::Vision,
                Capability::FunctionCalling
            ]
        );
        assert_eq!(chat.context_window, 32768);
        assert_eq!(chat.parameter_count, Some(4_000_000_000));
        assert_eq!(chat.provider, "flm");

        let embed = model_from_record("embed-gemma:300m", &json!({"label": ["embeddings"]}), "f");
        assert_eq!(embed.capabilities, vec![Capability::Embedding]);
        let asr = model_from_record(
            "whisper-v3:turbo",
            &json!({"label": ["audio", "realtime-transcription", "transcription"]}),
            "f",
        );
        assert_eq!(asr.capabilities, vec![Capability::Audio]);
        // A chat model that also takes audio stays a chat model.
        let omni = model_from_record(
            "gemma4-it:e2b",
            &json!({"label": ["audio", "chat-transcription"], "vlm": true}),
            "f",
        );
        assert!(omni.capabilities.contains(&Capability::Chat));
        let bare = model_from_record("lfm2:1.2b", &json!({"label": null}), "f");
        assert_eq!(
            bare.capabilities,
            vec![Capability::Chat, Capability::Completion]
        );
        assert_eq!(bare.context_window, 4096);
    }

    #[test]
    fn parameter_sizes() {
        assert_eq!(parse_parameter_size("1.8B"), Some(1_800_000_000));
        assert_eq!(parse_parameter_size("300M"), Some(300_000_000));
        assert_eq!(parse_parameter_size("35B-A3B"), None);
        assert_eq!(parse_parameter_size(""), None);
    }

    #[test]
    fn folder_scan_finds_models_with_a_config() {
        let dir = tempfile::tempdir().unwrap();
        let models_dir = dir.path().join("flm");
        let root = models_dir.join("models");
        std::fs::create_dir_all(root.join("Llama-3.2-1B-NPU2")).unwrap();
        std::fs::write(root.join("Llama-3.2-1B-NPU2/config.json"), "{}").unwrap();
        // Downloaded partially: no config.json yet.
        std::fs::create_dir_all(root.join("Qwen3-4B-NPU2")).unwrap();
        let catalog = dir.path().join("model_list.json");
        std::fs::write(
            &catalog,
            json!({
                "model_path": "models",
                "models": {
                    "llama3.2": {"1b": {"name": "Llama-3.2-1B-NPU2", "default_context_length": 131072}},
                    "qwen3": {"4b": {"name": "Qwen3-4B-NPU2"}}
                }
            })
            .to_string(),
        )
        .unwrap();
        let locations = Locations {
            models_dir: Some(models_dir.clone()),
            catalogs: vec![dir.path().join("missing.json"), catalog.clone()],
        };
        let found = scan_installed(&locations).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "llama3.2:1b");
        let models = models_from_entries(&found, "flm");
        assert_eq!(models[0].context_window, 131072);

        // No catalog, or no model folder: cannot tell.
        assert!(scan_installed(&Locations {
            models_dir: Some(models_dir),
            catalogs: vec![]
        })
        .is_none());
        assert!(scan_installed(&Locations {
            models_dir: Some(dir.path().join("nope")),
            catalogs: vec![catalog]
        })
        .is_none());
    }

    #[tokio::test]
    async fn remote_servers_are_not_inspected_locally() {
        assert!(InstalledModels::new()
            .detect("flm", "http://192.168.1.20:52625/v1")
            .await
            .is_none());
    }
}
