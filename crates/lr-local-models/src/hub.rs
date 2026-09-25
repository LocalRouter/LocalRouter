//! Hugging Face Hub client (plain reqwest, no `hf-hub`).
//!
//! Every method makes exactly the requests needed to answer the call; nothing
//! is fetched in the background. The optional bearer token is only sent to the
//! configured endpoint origin (and `https://huggingface.co`), never to CDN
//! hosts that downloads redirect to.

use std::sync::Arc;
use std::time::Duration;

use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::gguf::{self, GgufError, GgufHeader};
use crate::http;

/// Default Hub endpoint.
pub const DEFAULT_ENDPOINT: &str = "https://huggingface.co";

/// Timeout for API (non-download) requests.
const API_TIMEOUT: Duration = Duration::from_secs(30);
/// Upper bound on tree pages followed for one call.
const MAX_TREE_PAGES: usize = 200;
/// Fields requested with `expand[]` when searching.
const SEARCH_EXPAND: &[&str] = &[
    "downloads",
    "likes",
    "gated",
    "pipeline_tag",
    "library_name",
    "tags",
    "lastModified",
    "gguf",
];

/// Errors returned by the Hub client (and by downloads, which share it).
#[derive(Debug, Clone, thiserror::Error, Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum HubError {
    /// The repository is gated. `requires_login` is true when the request
    /// was anonymous (sign in first); false when the signed-in account has
    /// not been granted access yet.
    #[error("{}", gated_message(repo, message, *requires_login))]
    Gated {
        repo: String,
        message: String,
        requires_login: bool,
    },
    #[error("Not found on Hugging Face: {0}")]
    NotFound(String),
    #[error("{0} was not found on Hugging Face, or it is private and needs a signed-in account with access")]
    NotFoundOrPrivate(String),
    #[error("{}", rate_limited_message(*retry_after_secs))]
    RateLimited { retry_after_secs: Option<u64> },
    #[error("Hugging Face returned HTTP {status}: {message}")]
    Http { status: u16, message: String },
    #[error("Network error while contacting Hugging Face: {0}")]
    Network(String),
    #[error("Unexpected response from Hugging Face: {0}")]
    Parse(String),
    /// The caller passed something unusable (bad repo id, unsafe path, ...).
    #[error("{0}")]
    InvalidRequest(String),
    /// A local storage problem (not enough disk space, I/O error, ...).
    #[error("{0}")]
    Storage(String),
}

fn gated_message(repo: &str, message: &str, requires_login: bool) -> String {
    let base = if requires_login {
        format!("{repo} is a gated model. Sign in to Hugging Face and request access on huggingface.co/{repo} to download it.")
    } else {
        format!("{repo} is a gated model and your Hugging Face account does not have access yet. Request access on huggingface.co/{repo}.")
    };
    if message.is_empty() {
        base
    } else {
        format!("{base} ({message})")
    }
}

fn rate_limited_message(retry_after: Option<u64>) -> String {
    match retry_after {
        Some(s) => format!(
            "Hugging Face rate limit reached; try again in {s} seconds (signing in raises the limit)"
        ),
        None => "Hugging Face rate limit reached; try again later (signing in raises the limit)"
            .to_string(),
    }
}

/// Search parameters for [`HubClient::search`].
#[derive(Deserialize, Serialize, Default, Clone, Debug)]
pub struct HubSearch {
    pub query: Option<String>,
    /// Repeatable `filter=` values, e.g. `gguf`.
    pub filters: Vec<String>,
    pub pipeline_tag: Option<String>,
    /// `downloads`, `likes`, `trendingScore` or `lastModified` (descending).
    pub sort: Option<String>,
    /// Page size (default 30, max 100).
    pub limit: Option<u32>,
    /// The full `next` URL from a previous page's `Link` header.
    pub cursor: Option<String>,
}

/// One page of search results.
#[derive(Serialize, Clone, Debug)]
pub struct HubPage {
    pub models: Vec<HubModelSummary>,
    pub next_cursor: Option<String>,
}

/// A search result.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct HubModelSummary {
    pub id: String,
    pub author: Option<String>,
    pub downloads: u64,
    pub likes: u64,
    pub last_modified: Option<String>,
    /// `None` when not gated, else `"auto"` / `"manual"`.
    pub gated: Option<String>,
    pub pipeline_tag: Option<String>,
    pub library_name: Option<String>,
    pub tags: Vec<String>,
    /// `gguf.total` (parameter count).
    pub parameters: Option<u64>,
    pub architecture: Option<String>,
    pub context_length: Option<u64>,
}

/// Repository details from `/api/models/{repo}/revision/{rev}`.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct HubModelInfo {
    pub id: String,
    /// Commit SHA of the revision.
    pub sha: Option<String>,
    pub gated: Option<String>,
    /// `cardData.extra_gated_prompt`.
    pub gate_prompt: Option<String>,
    /// `cardData.license`.
    pub license: Option<String>,
    pub pipeline_tag: Option<String>,
    pub siblings: Vec<HubFile>,
}

/// A file in a repository.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct HubFile {
    pub path: String,
    pub size: Option<u64>,
    /// SHA-256 of LFS files (`lfs.sha256` / `lfs.oid`); `None` for small
    /// git-stored files.
    pub sha256: Option<String>,
}

/// The account behind a token (`/api/whoami-v2`).
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct HubUser {
    pub name: String,
    pub fullname: Option<String>,
    /// `auth.accessToken.role` (`read`, `write`, `fineGrained`).
    pub token_role: Option<String>,
}

struct Inner {
    endpoint: Url,
    base: String,
    client: reqwest::Client,
}

/// Hugging Face Hub client. Cheap to clone.
#[derive(Clone)]
pub struct HubClient {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for HubClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HubClient")
            .field("endpoint", &self.inner.base)
            .finish()
    }
}

impl Default for HubClient {
    fn default() -> Self {
        Self::new(DEFAULT_ENDPOINT)
    }
}

impl HubClient {
    /// Create a client for `endpoint` (e.g. `https://huggingface.co` or a
    /// mirror). An unparsable endpoint falls back to the default one.
    pub fn new(endpoint: &str) -> Self {
        let trimmed = endpoint.trim().trim_end_matches('/');
        let parsed = Url::parse(trimmed)
            .ok()
            .filter(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some());
        let (endpoint, base) = match parsed {
            Some(u) => (u, trimmed.to_string()),
            None => {
                tracing::warn!("invalid Hugging Face endpoint {trimmed:?}; using the default");
                (
                    Url::parse(DEFAULT_ENDPOINT).expect("default endpoint is valid"),
                    DEFAULT_ENDPOINT.to_string(),
                )
            }
        };
        Self {
            inner: Arc::new(Inner {
                endpoint,
                base,
                client: http::build_client(),
            }),
        }
    }

    /// The endpoint base URL (no trailing slash).
    pub fn endpoint(&self) -> &str {
        &self.inner.base
    }

    pub(crate) fn endpoint_url(&self) -> &Url {
        &self.inner.endpoint
    }

    /// The redirect-disabled HTTP client used by this Hub client.
    pub fn http_client(&self) -> &reqwest::Client {
        &self.inner.client
    }

    /// Web page of a repository (for "request access" links).
    pub fn repo_url(&self, repo: &str) -> String {
        format!("{}/{}", self.inner.base, repo)
    }

    /// `{endpoint}/{repo}/resolve/{rev}/{path}` with each path segment
    /// percent-encoded.
    pub fn resolve_url(&self, repo: &str, revision: &str, path: &str) -> String {
        let path = path
            .split('/')
            .map(|seg| urlencoding::encode(seg).into_owned())
            .collect::<Vec<_>>()
            .join("/");
        format!(
            "{}/{}/resolve/{}/{}",
            self.inner.base,
            repo,
            urlencoding::encode(revision),
            path
        )
    }

    fn parse_url(&self, s: &str) -> Result<Url, HubError> {
        Url::parse(s).map_err(|e| HubError::InvalidRequest(format!("bad URL: {e}")))
    }

    async fn get(&self, url: Url, token: Option<&str>) -> Result<http::Followed, HubError> {
        http::get_following(
            &self.inner.client,
            url,
            &self.inner.endpoint,
            token,
            &[],
            Some(API_TIMEOUT),
        )
        .await
    }

    async fn get_json(
        &self,
        url: Url,
        token: Option<&str>,
        repo: &str,
    ) -> Result<(Value, reqwest::header::HeaderMap), HubError> {
        let followed = self.get(url, token).await?;
        let resp = followed.response;
        if !resp.status().is_success() {
            return Err(http::error_from_response(resp, repo).await);
        }
        let headers = resp.headers().clone();
        let text = resp.text().await.map_err(http::network_error)?;
        let value = serde_json::from_str(&text).map_err(|e| HubError::Parse(e.to_string()))?;
        Ok((value, headers))
    }

    /// Search models.
    pub async fn search(&self, q: &HubSearch, token: Option<&str>) -> Result<HubPage, HubError> {
        let url = match q.cursor.as_deref().filter(|c| !c.is_empty()) {
            Some(cursor) => {
                let url = self.parse_url(cursor)?;
                if !http::same_origin(&url, &self.inner.endpoint) {
                    return Err(HubError::InvalidRequest(
                        "search cursor does not belong to the configured endpoint".into(),
                    ));
                }
                url
            }
            None => self.search_url(q)?,
        };
        let (value, headers) = self.get_json(url, token, "").await?;
        let items = value
            .as_array()
            .ok_or_else(|| HubError::Parse("search results are not a list".into()))?;
        let models = items.iter().filter_map(parse_summary).collect();
        let next_cursor = http::parse_next_link(&headers).filter(|next| {
            Url::parse(next)
                .map(|u| http::same_origin(&u, &self.inner.endpoint))
                .unwrap_or(false)
        });
        Ok(HubPage {
            models,
            next_cursor,
        })
    }

    fn search_url(&self, q: &HubSearch) -> Result<Url, HubError> {
        let mut url = self.parse_url(&format!("{}/api/models", self.inner.base))?;
        {
            let mut qp = url.query_pairs_mut();
            if let Some(query) = q.query.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                qp.append_pair("search", query);
            }
            for f in q.filters.iter().filter(|f| !f.is_empty()) {
                qp.append_pair("filter", f);
            }
            if let Some(tag) = q.pipeline_tag.as_deref().filter(|s| !s.is_empty()) {
                qp.append_pair("pipeline_tag", tag);
            }
            if let Some(sort) = q.sort.as_deref().filter(|s| !s.is_empty()) {
                qp.append_pair("sort", sort);
                qp.append_pair("direction", "-1");
            }
            let limit = q.limit.unwrap_or(30).clamp(1, 100);
            qp.append_pair("limit", &limit.to_string());
            for e in SEARCH_EXPAND {
                qp.append_pair("expand[]", e);
            }
        }
        Ok(url)
    }

    /// Repository info at a revision (default `main`), including file sizes
    /// and LFS SHA-256s.
    pub async fn model_info(
        &self,
        repo: &str,
        revision: Option<&str>,
        token: Option<&str>,
    ) -> Result<HubModelInfo, HubError> {
        validate_repo_id(repo)?;
        let rev = revision.filter(|r| !r.is_empty()).unwrap_or("main");
        let url = self.parse_url(&format!(
            "{}/api/models/{}/revision/{}?blobs=true",
            self.inner.base,
            repo,
            urlencoding::encode(rev)
        ))?;
        let (value, _) = self.get_json(url, token, repo).await?;
        parse_model_info(&value, repo)
    }

    /// All files of a repository at a revision (recursive, all pages).
    pub async fn tree(
        &self,
        repo: &str,
        revision: Option<&str>,
        token: Option<&str>,
    ) -> Result<Vec<HubFile>, HubError> {
        validate_repo_id(repo)?;
        let rev = revision.filter(|r| !r.is_empty()).unwrap_or("main");
        let mut url = self.parse_url(&format!(
            "{}/api/models/{}/tree/{}?recursive=true",
            self.inner.base,
            repo,
            urlencoding::encode(rev)
        ))?;
        let mut files = Vec::new();
        for _ in 0..MAX_TREE_PAGES {
            let (value, headers) = self.get_json(url, token, repo).await?;
            let items = value
                .as_array()
                .ok_or_else(|| HubError::Parse("tree is not a list".into()))?;
            files.extend(items.iter().filter_map(parse_tree_entry));
            match http::parse_next_link(&headers)
                .and_then(|n| Url::parse(&n).ok())
                .filter(|u| http::same_origin(u, &self.inner.endpoint))
            {
                Some(next) => url = next,
                None => return Ok(files),
            }
        }
        Ok(files)
    }

    /// Raw `README.md` of a repository (empty when the repo has none).
    pub async fn readme(
        &self,
        repo: &str,
        revision: Option<&str>,
        token: Option<&str>,
    ) -> Result<String, HubError> {
        validate_repo_id(repo)?;
        let rev = revision.filter(|r| !r.is_empty()).unwrap_or("main");
        let url = self.parse_url(&self.resolve_url(repo, rev, "README.md"))?;
        let resp = self.get(url, token).await?.response;
        if !resp.status().is_success() {
            return match http::error_from_response(resp, repo).await {
                HubError::NotFound(_) => Ok(String::new()),
                e => Err(e),
            };
        }
        resp.text().await.map_err(http::network_error)
    }

    /// The account a token belongs to.
    pub async fn whoami(&self, token: &str) -> Result<HubUser, HubError> {
        let url = self.parse_url(&format!("{}/api/whoami-v2", self.inner.base))?;
        let (value, _) = self.get_json(url, Some(token), "").await?;
        let name = value
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| HubError::Parse("whoami response has no name".into()))?
            .to_string();
        Ok(HubUser {
            name,
            fullname: str_field(&value, "fullname"),
            token_role: value
                .pointer("/auth/accessToken/role")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }

    /// Read the GGUF header of a file in a repository with Range requests
    /// (the token never reaches the CDN).
    pub async fn gguf_header(
        &self,
        repo: &str,
        revision: Option<&str>,
        path: &str,
        token: Option<&str>,
    ) -> Result<GgufHeader, GgufError> {
        validate_repo_id(repo)?;
        let rev = revision.filter(|r| !r.is_empty()).unwrap_or("main");
        let url = self.parse_url(&self.resolve_url(repo, rev, path))?;
        gguf::read_remote_header_with(&self.inner.client, url, &self.inner.endpoint, token, repo)
            .await
    }
}

/// Validate a repo id (`name` or `org/name`).
pub(crate) fn validate_repo_id(repo: &str) -> Result<(), HubError> {
    let segments: Vec<&str> = repo.split('/').collect();
    let ok = !repo.is_empty()
        && segments.len() <= 2
        && segments.iter().all(|s| {
            !s.is_empty()
                && *s != "."
                && *s != ".."
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        });
    if ok {
        Ok(())
    } else {
        Err(HubError::InvalidRequest(format!(
            "invalid Hugging Face repository id: {repo:?}"
        )))
    }
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn parse_gated(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::Bool(false) | Value::Null => None,
        Value::Bool(true) => Some("true".into()),
        Value::String(s) if s.is_empty() || s == "false" => None,
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

fn parse_summary(v: &Value) -> Option<HubModelSummary> {
    let id = str_field(v, "id").or_else(|| str_field(v, "modelId"))?;
    let author = str_field(v, "author").or_else(|| id.split_once('/').map(|(a, _)| a.to_string()));
    let gguf = v.get("gguf");
    Some(HubModelSummary {
        author,
        downloads: v.get("downloads").and_then(Value::as_u64).unwrap_or(0),
        likes: v.get("likes").and_then(Value::as_u64).unwrap_or(0),
        last_modified: str_field(v, "lastModified"),
        gated: parse_gated(v.get("gated")),
        pipeline_tag: str_field(v, "pipeline_tag"),
        library_name: str_field(v, "library_name"),
        tags: v
            .get("tags")
            .and_then(Value::as_array)
            .map(|t| {
                t.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        parameters: gguf.and_then(|g| g.get("total")).and_then(Value::as_u64),
        architecture: gguf.and_then(|g| str_field(g, "architecture")),
        context_length: gguf
            .and_then(|g| g.get("context_length"))
            .and_then(Value::as_u64),
        id,
    })
}

fn parse_model_info(v: &Value, repo: &str) -> Result<HubModelInfo, HubError> {
    let obj = v
        .as_object()
        .ok_or_else(|| HubError::Parse("model info is not an object".into()))?;
    let card = obj.get("cardData");
    let license = card.and_then(|c| c.get("license")).and_then(|l| match l {
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => {
            let parts: Vec<&str> = items.iter().filter_map(Value::as_str).collect();
            (!parts.is_empty()).then(|| parts.join(", "))
        }
        _ => None,
    });
    let siblings = obj
        .get("siblings")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|s| {
                    let path = str_field(s, "rfilename")?;
                    let lfs = s.get("lfs");
                    Some(HubFile {
                        path,
                        size: s
                            .get("size")
                            .and_then(Value::as_u64)
                            .or_else(|| lfs.and_then(|l| l.get("size")).and_then(Value::as_u64)),
                        sha256: lfs
                            .and_then(|l| str_field(l, "sha256").or_else(|| str_field(l, "oid"))),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(HubModelInfo {
        id: str_field(v, "id")
            .or_else(|| str_field(v, "modelId"))
            .unwrap_or_else(|| repo.to_string()),
        sha: str_field(v, "sha"),
        gated: parse_gated(obj.get("gated")),
        gate_prompt: card.and_then(|c| str_field(c, "extra_gated_prompt")),
        license,
        pipeline_tag: str_field(v, "pipeline_tag"),
        siblings,
    })
}

fn parse_tree_entry(v: &Value) -> Option<HubFile> {
    if v.get("type").and_then(Value::as_str) != Some("file") {
        return None;
    }
    let lfs = v.get("lfs");
    Some(HubFile {
        path: str_field(v, "path")?,
        size: v
            .get("size")
            .and_then(Value::as_u64)
            .or_else(|| lfs.and_then(|l| l.get("size")).and_then(Value::as_u64)),
        sha256: lfs.and_then(|l| str_field(l, "oid").or_else(|| str_field(l, "sha256"))),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const SEARCH_FIXTURE: &str = r#"[
      {"_id":"1","id":"unsloth/Qwen3-8B-GGUF","downloads":123456,"likes":321,
       "gated":false,"pipeline_tag":"text-generation","library_name":"transformers",
       "tags":["gguf","qwen3"],"lastModified":"2025-05-01T00:00:00.000Z",
       "gguf":{"total":8190735360,"architecture":"qwen3","context_length":40960,
               "chat_template":"{% huge template %}","bos_token":"<s>"}},
      {"_id":"2","id":"meta-llama/Llama-3.2-1B","downloads":5,"likes":1,
       "gated":"manual","tags":[]},
      {"_id":"3","modelId":"gpt2"}
    ]"#;

    #[test]
    fn parses_search_fixture() {
        let v: Value = serde_json::from_str(SEARCH_FIXTURE).unwrap();
        let models: Vec<_> = v
            .as_array()
            .unwrap()
            .iter()
            .filter_map(parse_summary)
            .collect();
        assert_eq!(models.len(), 3);
        let m = &models[0];
        assert_eq!(m.id, "unsloth/Qwen3-8B-GGUF");
        assert_eq!(m.author.as_deref(), Some("unsloth"));
        assert_eq!(m.downloads, 123456);
        assert_eq!(m.likes, 321);
        assert_eq!(m.gated, None);
        assert_eq!(m.parameters, Some(8190735360));
        assert_eq!(m.architecture.as_deref(), Some("qwen3"));
        assert_eq!(m.context_length, Some(40960));
        assert_eq!(m.tags, vec!["gguf", "qwen3"]);
        // The chat template is not carried over.
        assert!(!serde_json::to_string(m).unwrap().contains("huge template"));
        assert_eq!(models[1].gated.as_deref(), Some("manual"));
        assert_eq!(models[2].id, "gpt2");
        assert_eq!(models[2].author, None);
    }

    #[test]
    fn parses_model_info_and_tree() {
        let v: Value = serde_json::from_str(
            r#"{"id":"org/repo","sha":"abc","gated":"auto","pipeline_tag":"text-generation",
                "cardData":{"license":"apache-2.0","extra_gated_prompt":"Agree to terms"},
                "siblings":[
                  {"rfilename":"README.md","size":100},
                  {"rfilename":"m-Q4_K_M.gguf","size":5000,"lfs":{"sha256":"ff00","size":5000}}
                ]}"#,
        )
        .unwrap();
        let info = parse_model_info(&v, "org/repo").unwrap();
        assert_eq!(info.sha.as_deref(), Some("abc"));
        assert_eq!(info.gated.as_deref(), Some("auto"));
        assert_eq!(info.gate_prompt.as_deref(), Some("Agree to terms"));
        assert_eq!(info.license.as_deref(), Some("apache-2.0"));
        assert_eq!(info.siblings.len(), 2);
        assert_eq!(info.siblings[0].sha256, None);
        assert_eq!(info.siblings[1].sha256.as_deref(), Some("ff00"));
        assert_eq!(info.siblings[1].size, Some(5000));

        let entry: Value = serde_json::from_str(
            r#"{"type":"file","path":"a/b.gguf","size":10,"oid":"gitsha","lfs":{"oid":"sha256hex","size":10}}"#,
        )
        .unwrap();
        let f = parse_tree_entry(&entry).unwrap();
        assert_eq!(f.path, "a/b.gguf");
        assert_eq!(f.sha256.as_deref(), Some("sha256hex"));
        let dir: Value = serde_json::from_str(r#"{"type":"directory","path":"a"}"#).unwrap();
        assert!(parse_tree_entry(&dir).is_none());
    }

    #[test]
    fn resolve_url_encodes_segments() {
        let hub = HubClient::new("https://huggingface.co/");
        assert_eq!(hub.endpoint(), "https://huggingface.co");
        assert_eq!(
            hub.resolve_url("org/repo", "main", "sub dir/file#1.gguf"),
            "https://huggingface.co/org/repo/resolve/main/sub%20dir/file%231.gguf"
        );
        assert_eq!(
            hub.resolve_url("org/repo", "refs/pr/1", "a.gguf"),
            "https://huggingface.co/org/repo/resolve/refs%2Fpr%2F1/a.gguf"
        );
        assert_eq!(HubClient::new("not a url").endpoint(), DEFAULT_ENDPOINT);
    }

    #[test]
    fn repo_id_validation() {
        assert!(validate_repo_id("org/repo").is_ok());
        assert!(validate_repo_id("gpt2").is_ok());
        assert!(validate_repo_id("Org_1/re.po-x").is_ok());
        for bad in ["", "a/b/c", "../x", "a/..", "a b/c", "/a", "a/"] {
            assert!(validate_repo_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn error_display_is_friendly() {
        let e = HubError::Gated {
            repo: "a/b".into(),
            message: String::new(),
            requires_login: true,
        };
        assert!(e.to_string().contains("Sign in"));
        let e = HubError::RateLimited {
            retry_after_secs: Some(30),
        };
        assert!(e.to_string().contains("30 seconds"));
        let json = serde_json::to_value(HubError::NotFound("x".into())).unwrap();
        assert_eq!(json["kind"], "not_found");
    }

    #[tokio::test]
    async fn search_builds_query_and_pages() {
        let server = MockServer::start().await;
        let next = format!("{}/api/models?cursor=PAGE2&limit=2", server.uri());
        Mock::given(method("GET"))
            .and(path("/api/models"))
            .and(query_param("search", "qwen"))
            .and(query_param("filter", "gguf"))
            .and(query_param("sort", "downloads"))
            .and(query_param("direction", "-1"))
            .and(query_param("limit", "2"))
            .and(query_param("expand[]", "gguf"))
            .and(header("authorization", "Bearer tok"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("link", format!("<{next}>; rel=\"next\"").as_str())
                    .set_body_string(SEARCH_FIXTURE),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/models"))
            .and(query_param("cursor", "PAGE2"))
            .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
            .mount(&server)
            .await;

        let hub = HubClient::new(&server.uri());
        let q = HubSearch {
            query: Some("qwen".into()),
            filters: vec!["gguf".into()],
            sort: Some("downloads".into()),
            limit: Some(2),
            ..Default::default()
        };
        let page = hub.search(&q, Some("tok")).await.unwrap();
        assert_eq!(page.models.len(), 3);
        assert_eq!(page.next_cursor.as_deref(), Some(next.as_str()));

        let page2 = hub
            .search(
                &HubSearch {
                    cursor: page.next_cursor.clone(),
                    ..Default::default()
                },
                None,
            )
            .await
            .unwrap();
        assert!(page2.models.is_empty());
        assert!(page2.next_cursor.is_none());

        // A cursor for another origin is rejected (it would receive the token).
        let err = hub
            .search(
                &HubSearch {
                    cursor: Some("https://evil.example/api/models".into()),
                    ..Default::default()
                },
                Some("tok"),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, HubError::InvalidRequest(_)));
    }

    #[tokio::test]
    async fn model_info_errors_are_mapped() {
        let server = MockServer::start().await;
        Mock::given(path("/api/models/org/gated/revision/main"))
            .respond_with(
                ResponseTemplate::new(401)
                    .insert_header("x-error-code", "GatedRepo")
                    .insert_header(
                        "x-error-message",
                        "Access to model org/gated is restricted.",
                    ),
            )
            .mount(&server)
            .await;
        Mock::given(path("/api/models/org/private/revision/main"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        Mock::given(path("/api/models/org/busy/revision/main"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "7"))
            .mount(&server)
            .await;
        Mock::given(path("/api/models/org/ok/revision/v1.0"))
            .and(query_param("blobs", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"id":"org/ok","sha":"0123456789abcdef0123456789abcdef01234567","siblings":[]}"#,
            ))
            .mount(&server)
            .await;
        let hub = HubClient::new(&server.uri());
        assert!(matches!(
            hub.model_info("org/gated", None, None).await,
            Err(HubError::Gated {
                requires_login: true,
                ..
            })
        ));
        assert!(matches!(
            hub.model_info("org/private", None, None).await,
            Err(HubError::NotFoundOrPrivate(_))
        ));
        assert!(matches!(
            hub.model_info("org/busy", None, None).await,
            Err(HubError::RateLimited {
                retry_after_secs: Some(7)
            })
        ));
        let info = hub.model_info("org/ok", Some("v1.0"), None).await.unwrap();
        assert_eq!(info.id, "org/ok");
        assert!(matches!(
            hub.model_info("../etc", None, None).await,
            Err(HubError::InvalidRequest(_))
        ));
    }

    #[tokio::test]
    async fn tree_follows_cursor_pages() {
        let server = MockServer::start().await;
        let next = format!(
            "{}/api/models/org/repo/tree/main?recursive=true&cursor=P2",
            server.uri()
        );
        Mock::given(path("/api/models/org/repo/tree/main"))
            .and(query_param("cursor", "P2"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"[{"type":"file","path":"b.gguf","size":2,"lfs":{"oid":"bb","size":2}}]"#,
            ))
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(path("/api/models/org/repo/tree/main"))
            .and(query_param("recursive", "true"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("link", format!("<{next}>; rel=\"next\"").as_str())
                    .set_body_string(
                        r#"[{"type":"directory","path":"d"},{"type":"file","path":"a.json","size":1}]"#,
                    ),
            )
            .with_priority(5)
            .mount(&server)
            .await;
        let hub = HubClient::new(&server.uri());
        let files = hub.tree("org/repo", None, None).await.unwrap();
        assert_eq!(
            files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            vec!["a.json", "b.gguf"]
        );
        assert_eq!(files[1].sha256.as_deref(), Some("bb"));
    }

    #[tokio::test]
    async fn readme_and_whoami() {
        let server = MockServer::start().await;
        Mock::given(path("/org/repo/resolve/main/README.md"))
            .respond_with(ResponseTemplate::new(200).set_body_string("# Hello"))
            .mount(&server)
            .await;
        Mock::given(path("/org/empty/resolve/main/README.md"))
            .respond_with(ResponseTemplate::new(404).insert_header("x-error-code", "EntryNotFound"))
            .mount(&server)
            .await;
        Mock::given(path("/api/whoami-v2"))
            .and(header("authorization", "Bearer hf_good"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"type":"user","name":"alice","fullname":"Alice A","auth":{"type":"access_token","accessToken":{"displayName":"t","role":"fineGrained"}}}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(path("/api/whoami-v2"))
            .respond_with(
                ResponseTemplate::new(401)
                    .set_body_string(r#"{"error":"Invalid credentials in Authorization header"}"#),
            )
            .mount(&server)
            .await;
        let hub = HubClient::new(&server.uri());
        assert_eq!(hub.readme("org/repo", None, None).await.unwrap(), "# Hello");
        assert_eq!(hub.readme("org/empty", None, None).await.unwrap(), "");
        let user = hub.whoami("hf_good").await.unwrap();
        assert_eq!(user.name, "alice");
        assert_eq!(user.fullname.as_deref(), Some("Alice A"));
        assert_eq!(user.token_role.as_deref(), Some("fineGrained"));
        assert!(hub.whoami("hf_bad").await.is_err());
    }

    #[tokio::test]
    async fn remote_gguf_header_via_cdn_without_token() {
        use crate::gguf::test_support::{GgufBuilder, Val};
        use crate::test_util::RangeResponder;

        let hub_server = MockServer::start().await;
        let cdn = MockServer::start().await;
        // A header larger than one 2 MiB chunk forces several range requests.
        let tokens: Vec<String> = (0..200_000).map(|i| format!("tok{i:07}")).collect();
        let bytes = GgufBuilder::decoder("llama")
            .kv("tokenizer.ggml.tokens", Val::StrArray(tokens))
            .build_file(4 * 1024 * 1024);
        assert!(bytes.len() > 4 * 1024 * 1024);
        Mock::given(path("/org/repo/resolve/main/m.gguf"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/blob/m.gguf", cdn.uri()).as_str()),
            )
            .mount(&hub_server)
            .await;
        Mock::given(path("/blob/m.gguf"))
            .respond_with(RangeResponder::new(bytes.clone()))
            .mount(&cdn)
            .await;
        let hub = HubClient::new(&hub_server.uri());
        let h = hub
            .gguf_header("org/repo", None, "m.gguf", Some("secret"))
            .await
            .unwrap();
        assert_eq!(h.array_lengths["tokenizer.ggml.tokens"], 200_000);
        let cdn_reqs = cdn.received_requests().await.unwrap();
        assert!(cdn_reqs.len() >= 2);
        assert!(cdn_reqs
            .iter()
            .all(|r| !r.headers.contains_key("authorization")));
        let hub_reqs = hub_server.received_requests().await.unwrap();
        assert_eq!(hub_reqs.len(), 1, "later ranges reuse the CDN URL");
        assert!(hub_reqs[0].headers.contains_key("authorization"));

        // The standalone reader against a server that ignores Range.
        let plain = MockServer::start().await;
        let small = GgufBuilder::decoder("qwen3").build_file(100);
        Mock::given(path("/f.gguf"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(small))
            .mount(&plain)
            .await;
        let client = reqwest::Client::new();
        let h = gguf::read_remote_header(&client, &format!("{}/f.gguf", plain.uri()), None)
            .await
            .unwrap();
        assert_eq!(h.architecture(), Some("qwen3"));

        // Truncated remote file.
        let trunc = MockServer::start().await;
        let full = GgufBuilder::decoder("qwen3").build();
        Mock::given(path("/t.gguf"))
            .respond_with(RangeResponder::new(full[..full.len() - 4].to_vec()))
            .mount(&trunc)
            .await;
        assert!(matches!(
            gguf::read_remote_header(&client, &format!("{}/t.gguf", trunc.uri()), None).await,
            Err(GgufError::Invalid(_))
        ));
    }
}
