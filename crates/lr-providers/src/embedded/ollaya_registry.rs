//! Ollaya's model library as published in its GitHub repository, which adds
//! models to the built-in list in [`super::ollaya::LIBRARY`].
//!
//! Ollaya has no endpoint that lists its library. Its repository does: every
//! release tag carries `registry/v2/library/<model>/manifests/<tag>`, the
//! manifests `ollaya.dev` serves. Each manifest's config blob describes the
//! model (description, context length).
//!
//! A model's config does not say which Ollaya it needs, but the release tags
//! do: a model in the tree of the installed engine's tag runs on that engine,
//! and a model only in a newer release's tree needs that release.
//!
//! One refresh costs at most three GitHub API calls (the latest release and
//! one or two trees); manifests and configs come from
//! `raw.githubusercontent.com` and `ollaya.dev`, which have no API quota.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

use serde_json::Value;

/// Where the library is read from (tests point it at a mock server).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrySource {
    /// GitHub REST API base.
    pub api_base: String,
    /// Raw file base: `{raw_base}/{repo}/{tag}/{path}`.
    pub raw_base: String,
    /// `owner/name`.
    pub repo: String,
}

impl Default for RegistrySource {
    fn default() -> Self {
        Self {
            api_base: "https://api.github.com".to_string(),
            raw_base: "https://raw.githubusercontent.com".to_string(),
            repo: lr_engines::OLLAYA_REPO.to_string(),
        }
    }
}

/// A model the registry lists that the built-in list does not.
#[derive(Debug, Clone, PartialEq)]
pub struct RegistryModel {
    /// Ollaya name, `model:tag`.
    pub id: String,
    pub name: String,
    /// Total download (config and layers).
    pub size_bytes: u64,
    pub context: Option<u32>,
    pub description: Option<String>,
}

/// What the registry adds for one engine version.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RegistryView {
    /// The engine release this view was computed for, e.g. `v0.9.0`.
    pub engine_tag: String,
    /// Ollaya's latest release.
    pub latest_tag: String,
    /// Models not in the built-in list, those the engine runs first.
    pub added: Vec<RegistryModel>,
    /// Models only a newer Ollaya serves: id → the release that has them.
    pub needs_newer: HashMap<String, String>,
}

/// `v0.9.0`, `0.9.0`, `0.9.0-rc.1` → `(0, 9, 0)`.
pub fn parse_version(tag: &str) -> Option<(u64, u64, u64)> {
    let core = tag.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    Some((
        parts.next()??,
        parts.next()??,
        parts.next().flatten().unwrap_or(0),
    ))
}

/// `jeb:27b` → `Jeb 27B`, `clef:flash` → `Clef Flash`.
pub fn display_name(id: &str) -> String {
    let (model, tag) = id.rsplit_once(':').unwrap_or((id, ""));
    let model = model.rsplit('/').next().unwrap_or(model);
    let cap = |s: &str| {
        let mut c = s.chars();
        c.next()
            .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
            .unwrap_or_default()
    };
    let tag = tag
        .split('-')
        .map(|part| {
            // Sizes read as sizes: 9b → 9B, 0.8b → 0.8B, e4b → E4B.
            if part.ends_with('b') && part.chars().any(|c| c.is_ascii_digit()) {
                part.to_uppercase()
            } else {
                cap(part)
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    if tag.is_empty() || tag == "Latest" {
        cap(model)
    } else {
        format!("{} {tag}", cap(model))
    }
}

/// Model ids in one tree: `model:tag` for every manifest, without the
/// precision variants (`-fp16`, `-fp32`) of a listed model and without
/// `latest` where the model has other tags (it aliases one of them).
pub fn ids_from_tree(tree: &Value) -> Vec<String> {
    let mut tags: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for entry in tree
        .get("tree")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if entry.get("type").and_then(Value::as_str) != Some("blob") {
            continue;
        }
        let Some(path) = entry.get("path").and_then(Value::as_str) else {
            continue;
        };
        let parts: Vec<&str> = path.split('/').collect();
        if let ["registry", "v2", "library", model, "manifests", tag] = parts[..] {
            tags.entry(model.to_lowercase())
                .or_default()
                .push(tag.to_lowercase());
        }
    }
    let mut ids = Vec::new();
    for (model, tags) in tags {
        let others = tags.iter().any(|t| t != "latest");
        for tag in &tags {
            if tag.ends_with("-fp16") || tag.ends_with("-fp32") {
                continue;
            }
            if tag == "latest" && others {
                continue;
            }
            ids.push(format!("{model}:{tag}"));
        }
    }
    ids
}

fn fail(what: &str, e: impl std::fmt::Display) -> String {
    format!("{what}: {e}")
}

async fn get_json(
    http: &reqwest::Client,
    url: &str,
    github_api: bool,
) -> Result<Option<Value>, String> {
    let mut req = http.get(url).timeout(Duration::from_secs(30));
    if github_api {
        req = req.header(reqwest::header::ACCEPT, "application/vnd.github+json");
    }
    let resp = req.send().await.map_err(|e| fail(url, e))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let resp = resp.error_for_status().map_err(|e| fail(url, e))?;
    resp.json::<Value>()
        .await
        .map(Some)
        .map_err(|e| fail(url, e))
}

async fn tree_ids(
    http: &reqwest::Client,
    src: &RegistrySource,
    tag: &str,
) -> Result<Option<Vec<String>>, String> {
    let url = format!(
        "{}/repos/{}/git/trees/{tag}?recursive=1",
        src.api_base.trim_end_matches('/'),
        src.repo
    );
    Ok(get_json(http, &url, true).await?.map(|t| ids_from_tree(&t)))
}

/// Manifest and config of one model at `tag`.
async fn describe(
    http: &reqwest::Client,
    src: &RegistrySource,
    tag: &str,
    id: &str,
) -> Result<RegistryModel, String> {
    let (model, model_tag) = id.split_once(':').unwrap_or((id, "latest"));
    let url = format!(
        "{}/{}/{tag}/registry/v2/library/{model}/manifests/{model_tag}",
        src.raw_base.trim_end_matches('/'),
        src.repo
    );
    let manifest = get_json(http, &url, false)
        .await?
        .ok_or_else(|| fail(&url, "not found"))?;
    let size = |x: &Value| x.get("size").and_then(Value::as_u64).unwrap_or(0);
    let size_bytes = manifest.get("config").map(size).unwrap_or(0)
        + manifest
            .get("layers")
            .and_then(Value::as_array)
            .map(|l| l.iter().map(size).sum())
            .unwrap_or(0);
    // The config blob's own URL, as Ollaya pulls it.
    let config_url = manifest
        .pointer("/config/urls/0")
        .and_then(Value::as_str)
        .map(str::to_string);
    let config = match config_url {
        Some(url) => get_json(http, &url, false).await.ok().flatten(),
        None => None,
    };
    let config = config.unwrap_or(Value::Null);
    Ok(RegistryModel {
        id: id.to_string(),
        name: display_name(id),
        size_bytes,
        context: config
            .get("context_length")
            .and_then(Value::as_u64)
            .filter(|c| *c > 0)
            .map(|c| c as u32),
        description: config
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

/// Read the library for an engine at `engine_tag`. `built_in` says which ids
/// the built-in list already describes (they are not fetched).
pub async fn fetch(
    http: &reqwest::Client,
    src: &RegistrySource,
    engine_tag: &str,
    built_in: &(dyn Fn(&str) -> bool + Sync),
) -> Result<RegistryView, String> {
    let latest_url = format!(
        "{}/repos/{}/releases/latest",
        src.api_base.trim_end_matches('/'),
        src.repo
    );
    let latest_tag = get_json(http, &latest_url, true)
        .await?
        .and_then(|r| {
            r.get("tag_name")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .ok_or_else(|| fail(&latest_url, "no latest release"))?;

    // The engine's own tag is missing for builds that are not releases;
    // then the latest release stands in for it.
    let engine_ids = tree_ids(http, src, engine_tag).await?;
    let engine_found = engine_ids.is_some();
    let latest_ids = if latest_tag == engine_tag {
        engine_ids.clone()
    } else {
        tree_ids(http, src, &latest_tag).await?
    }
    .unwrap_or_default();
    let compatible_tag = if engine_found {
        engine_tag
    } else {
        latest_tag.as_str()
    };
    let compatible: Vec<String> = engine_ids.unwrap_or_else(|| latest_ids.clone());
    let compatible_set: HashSet<&str> = compatible.iter().map(String::as_str).collect();

    let newer = match (parse_version(&latest_tag), parse_version(engine_tag)) {
        (Some(l), Some(e)) => engine_found && l > e,
        _ => false,
    };
    let needs_newer: HashMap<String, String> = if newer {
        latest_ids
            .iter()
            .filter(|id| !compatible_set.contains(id.as_str()))
            .map(|id| (id.clone(), latest_tag.clone()))
            .collect()
    } else {
        HashMap::new()
    };

    let mut wanted: Vec<(String, String)> = compatible
        .iter()
        .filter(|id| !built_in(id))
        .map(|id| (id.clone(), compatible_tag.to_string()))
        .collect();
    let mut newer_ids: Vec<&String> = needs_newer.keys().filter(|id| !built_in(id)).collect();
    newer_ids.sort();
    wanted.extend(
        newer_ids
            .into_iter()
            .map(|id| (id.clone(), latest_tag.clone())),
    );

    let described =
        futures::future::join_all(wanted.iter().map(|(id, tag)| describe(http, src, tag, id)))
            .await;
    let mut added = Vec::new();
    for (result, (id, _)) in described.into_iter().zip(&wanted) {
        match result {
            Ok(m) => added.push(m),
            Err(e) => tracing::debug!("Ollaya registry: skipping {id}: {e}"),
        }
    }
    Ok(RegistryView {
        engine_tag: engine_tag.to_string(),
        latest_tag,
        added,
        needs_newer,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn versions_and_names() {
        assert_eq!(parse_version("v0.9.0"), Some((0, 9, 0)));
        assert_eq!(parse_version("0.10.2-rc.1"), Some((0, 10, 2)));
        assert_eq!(parse_version("nightly"), None);
        assert!(parse_version("v0.10.0") > parse_version("v0.9.0"));
        assert_eq!(display_name("jeb:27b"), "Jeb 27B");
        assert_eq!(display_name("clef:flash"), "Clef Flash");
        assert_eq!(display_name("winnow:e4b"), "Winnow E4B");
        assert_eq!(display_name("decider:2b-vision"), "Decider 2B Vision");
        assert_eq!(display_name("solo:latest"), "Solo");
    }

    fn tree(paths: &[&str]) -> Value {
        let mut entries: Vec<Value> = paths
            .iter()
            .map(|p| json!({"path": format!("registry/v2/library/{p}"), "type": "blob"}))
            .collect();
        entries.push(json!({"path": "registry/v2/library/laya", "type": "tree"}));
        entries.push(json!({"path": "README.md", "type": "blob"}));
        json!({"tree": entries, "truncated": false})
    }

    #[test]
    fn tree_ids_skip_aliases_and_precision_variants() {
        let ids = ids_from_tree(&tree(&[
            "laya/manifests/en",
            "laya/manifests/en-fp16",
            "laya/manifests/en-fp32",
            "laya/manifests/latest",
            "solo/manifests/latest",
            "jeb/manifests/9b",
        ]));
        assert_eq!(ids, vec!["jeb:9b", "laya:en", "solo:latest"]);
    }

    /// A GitHub + ollaya.dev stand-in: the latest release is v0.10.0; the
    /// engine's v0.9.0 has `acme:2b` (not built in) and `laya:en` (built
    /// in); v0.10.0 adds `zeta:1b`.
    pub(crate) async fn mock_registry() -> MockServer {
        let server = MockServer::start().await;
        let uri = server.uri();
        let json_at = |p: String, body: Value| {
            Mock::given(method("GET"))
                .and(path(p))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
        };
        json_at(
            "/repos/ollaya-dev/ollaya/releases/latest".into(),
            json!({"tag_name": "v0.10.0"}),
        )
        .mount(&server)
        .await;
        json_at(
            "/repos/ollaya-dev/ollaya/git/trees/v0.9.0".into(),
            tree(&[
                "laya/manifests/en",
                "acme/manifests/2b",
                "acme/manifests/latest",
            ]),
        )
        .mount(&server)
        .await;
        json_at(
            "/repos/ollaya-dev/ollaya/git/trees/v0.10.0".into(),
            tree(&[
                "laya/manifests/en",
                "acme/manifests/2b",
                "zeta/manifests/1b",
            ]),
        )
        .mount(&server)
        .await;
        for (tag, model, model_tag, cfg) in [
            ("v0.9.0", "acme", "2b", "c-acme"),
            ("v0.10.0", "zeta", "1b", "c-zeta"),
        ] {
            json_at(
                format!("/raw/ollaya-dev/ollaya/{tag}/registry/v2/library/{model}/manifests/{model_tag}"),
                json!({
                    "config": {"size": 100, "urls": [format!("{uri}/blobs/{cfg}")]},
                    "layers": [{"size": 2_000_000_000u64}, {"size": 900}]
                }),
            )
            .mount(&server)
            .await;
            json_at(
                format!("/blobs/{cfg}"),
                json!({"description": format!("The {model} model."), "context_length": 2048}),
            )
            .mount(&server)
            .await;
        }
        server
    }

    pub(crate) fn source(server: &MockServer) -> RegistrySource {
        RegistrySource {
            api_base: server.uri(),
            raw_base: format!("{}/raw", server.uri()),
            repo: "ollaya-dev/ollaya".into(),
        }
    }

    #[tokio::test]
    async fn adds_models_and_flags_those_needing_a_newer_engine() {
        let server = mock_registry().await;
        let view = fetch(&reqwest::Client::new(), &source(&server), "v0.9.0", &|id| {
            id == "laya:en"
        })
        .await
        .unwrap();
        assert_eq!(view.engine_tag, "v0.9.0");
        assert_eq!(view.latest_tag, "v0.10.0");
        let ids: Vec<&str> = view.added.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["acme:2b", "zeta:1b"]);
        let acme = &view.added[0];
        assert_eq!(acme.name, "Acme 2B");
        assert_eq!(acme.size_bytes, 2_000_001_000);
        assert_eq!(acme.context, Some(2048));
        assert_eq!(acme.description.as_deref(), Some("The acme model."));
        assert_eq!(
            view.needs_newer,
            HashMap::from([("zeta:1b".to_string(), "v0.10.0".to_string())])
        );
    }

    #[tokio::test]
    async fn an_engine_on_the_latest_release_needs_nothing_newer() {
        let server = mock_registry().await;
        let view = fetch(
            &reqwest::Client::new(),
            &source(&server),
            "v0.10.0",
            &|_| false,
        )
        .await
        .unwrap();
        assert!(view.needs_newer.is_empty());
        assert!(view.added.iter().any(|m| m.id == "zeta:1b"));
    }

    #[tokio::test]
    async fn an_engine_that_is_no_release_reads_the_latest_one() {
        let server = mock_registry().await;
        // No tree for this tag: the latest release's library stands in.
        let view = fetch(
            &reqwest::Client::new(),
            &source(&server),
            "v0.9.9-dev",
            &|_| false,
        )
        .await
        .unwrap();
        assert!(view.needs_newer.is_empty());
        assert!(view.added.iter().any(|m| m.id == "zeta:1b"));
    }

    /// Against GitHub: the built-in list covers the pinned release, and an
    /// older engine is told which models need a newer one
    /// (`cargo test -p lr-providers -- --ignored real_ollaya_registry`).
    #[tokio::test]
    #[ignore = "reads Ollaya's repository on GitHub"]
    async fn real_ollaya_registry() {
        let built_in = |id: &str| crate::embedded::ollaya::LIBRARY.iter().any(|m| m.id == id);
        let http = reqwest::Client::builder()
            .user_agent("LocalRouter-tests")
            .build()
            .unwrap();
        let src = RegistrySource::default();
        let pinned = fetch(&http, &src, lr_engines::OLLAYA_VERSION, &built_in)
            .await
            .unwrap();
        let behind: Vec<&str> = pinned
            .added
            .iter()
            .filter(|m| !pinned.needs_newer.contains_key(&m.id))
            .map(|m| m.id.as_str())
            .collect();
        assert!(
            behind.is_empty(),
            "the pinned release has models the built-in list lacks: {behind:?}"
        );
        let old = fetch(&http, &src, "v0.7.5", &built_in).await.unwrap();
        assert_eq!(
            old.needs_newer.get("nimble:9b").map(String::as_str),
            Some(pinned.latest_tag.as_str())
        );
        assert!(!old.needs_newer.contains_key("laya:en"));
    }

    #[tokio::test]
    async fn an_unreachable_registry_is_an_error() {
        let server = MockServer::start().await;
        let err = fetch(&reqwest::Client::new(), &source(&server), "v0.9.0", &|_| {
            false
        })
        .await
        .unwrap_err();
        assert!(err.contains("releases/latest"), "{err}");
    }
}
