//! Keeps tool configs that were set up for the HTTPS inspection proxy in step
//! with LocalRouter.
//!
//! Putting the proxy into a tool's config is an explicit user action
//! ([`proxy_setup::apply`] behind "Configure"). Once it is there it can go
//! stale without the user touching anything: the proxy moves to another port,
//! the client's secret is rotated, the client leaves proxy mode, or the client
//! is deleted. A stale entry is not cosmetic — the tool keeps sending its
//! traffic to a listener that is gone or that now rejects it.
//!
//! This module reconciles that. Which client a tool config was written for is
//! recorded in a small state file (`proxy-client-sync.json` in the data
//! directory: client, template and a hash of the URL written, never the URL
//! with its secret), together with every (client, template) pair seen, so a
//! client deleted just before the app quit is still cleaned up at the next
//! start. A record only counts while the file still carries the client's
//! URL marker (`://<client_id>:`, the start of the URL's userinfo); files
//! configured before the state file existed are recognized by that marker
//! alone. It never configures a tool that was not already configured for
//! that client.
//!
//! Only the proxy URL and the path of the public root CA certificate are ever
//! written to a tool's config.

use super::proxy::ProxyService;
use super::proxy_setup::{self, ProxyPlan};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// One client's claim on a tool config file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub client_id: String,
    pub template_id: String,
    /// The client still exists, still uses this template, and is in proxy mode.
    pub wants_proxy: bool,
}

/// What a tool config file's content says about LocalRouter's proxy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing of ours to maintain (never configured, or configured for a
    /// client this file's tool does not belong to).
    Untouched,
    /// Configured for candidate `.0`, which is still in proxy mode: keep it,
    /// and bring the URL up to date if it changed.
    Owned(usize),
    /// Configured for candidate `.0`, which no longer proxies through this
    /// tool (left proxy mode, changed template, or was deleted): remove.
    Stale(usize),
    /// Names LocalRouter's root CA, but no existing client's proxy URL, and no
    /// proxy-mode client uses this tool: a leftover to remove. Covers tools
    /// whose config only ever holds the CA path (Goose).
    Orphaned,
}

/// The substring a proxy URL for `client_id` always contains (the start of
/// its userinfo). Client ids are UUIDs, so this does not match by accident.
fn url_marker(client_id: &str) -> String {
    format!("://{client_id}:")
}

/// Classify a tool config file.
///
/// `candidates` are the known (client, template) pairs whose plan writes this
/// file; `existing_client_ids` is every client currently in the config,
/// whatever its template — a URL for any of them means a person set this up
/// (possibly by hand, for a client with a different template) and it is not a
/// leftover.
pub fn classify(
    content: &str,
    ca_cert_path: &str,
    candidates: &[Candidate],
    existing_client_ids: &[String],
) -> Verdict {
    let configured_for = |c: &Candidate| content.contains(&url_marker(&c.client_id));

    if let Some(i) = candidates
        .iter()
        .position(|c| c.wants_proxy && configured_for(c))
    {
        return Verdict::Owned(i);
    }
    if let Some(i) = candidates
        .iter()
        .position(|c| !c.wants_proxy && configured_for(c))
    {
        return Verdict::Stale(i);
    }
    // The combined bundle counts as ours too (Codex and Aider configs name
    // it instead of the root CA).
    let bundle = super::ca_bundle::combined_bundle_path()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let names_our_ca = [ca_cert_path, bundle.as_str()]
        .iter()
        .any(|path| !path.is_empty() && content.contains(path));
    let tool_still_proxied = candidates.iter().any(|c| c.wants_proxy);
    let names_an_existing_client = existing_client_ids
        .iter()
        .any(|id| content.contains(&url_marker(id)));
    if names_our_ca && !tool_still_proxied && !names_an_existing_client {
        return Verdict::Orphaned;
    }
    Verdict::Untouched
}

/// Whether a file configured for a client must be rewritten: only when the
/// proxy is running (so there is a URL to write) and the file does not
/// already hold exactly that URL.
pub fn needs_refresh(content: &str, current_url: Option<&str>) -> bool {
    current_url.is_some_and(|url| !content.contains(url))
}

/// What one reconcile pass did to a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome {
    /// Rewrote the proxy URL for this client.
    Refreshed { file: PathBuf, client_id: String },
    /// Removed LocalRouter's proxy configuration.
    Removed { file: PathBuf },
    /// Tried and failed; the file is as it was.
    Failed { file: PathBuf, error: String },
}

/// What LocalRouter wrote into one tool config file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FileRecord {
    pub client_id: String,
    pub template_id: String,
    /// SHA-256 of the proxy URL written. The URL itself embeds the client
    /// secret and is never stored.
    pub url_sha256: String,
    /// The combined CA bundle path written, for plans that use one. A record
    /// that does not match the plan's bundle (e.g. one written before Codex
    /// moved to the bundle) makes the next pass rewrite the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_bundle: Option<String>,
}

/// Reconcile state, persisted so it survives a restart: a client deleted
/// just before the app quit is still cleaned up on the next start.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SyncState {
    /// Every (client id, template id) pair seen. Deleted clients and changed
    /// templates stay in here until their tool config has been dealt with.
    #[serde(default)]
    pub seen: BTreeSet<(String, String)>,
    /// Tool config file -> what was written into it, by "Configure" or by a
    /// refresh. This, not a substring of the file, says which client a file
    /// was configured for.
    #[serde(default)]
    pub files: BTreeMap<PathBuf, FileRecord>,
}

/// Name of the state file in the data directory.
const STATE_FILE: &str = "proxy-client-sync.json";

/// Serializes every read-modify-write of a state file. Taken before
/// [`super::EXTERNAL_CONFIG_LOCK`] whenever both are held.
static STATE_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

/// The production state file (`<data dir>/proxy-client-sync.json`).
pub fn default_state_path() -> Option<PathBuf> {
    lr_utils::paths::config_dir()
        .ok()
        .map(|d| d.join(STATE_FILE))
}

impl SyncState {
    /// Load `path`; a missing or unreadable file is an empty state (the
    /// substring fallback still finds configs written before it existed).
    pub fn load(path: &std::path::Path) -> Self {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                tracing::warn!("Ignoring unreadable proxy sync state {:?}: {e}", path);
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// Write `path` atomically.
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        let part = path.with_extension("json.part");
        std::fs::write(&part, bytes)?;
        std::fs::rename(&part, path)
    }
}

fn url_hash(url: &str) -> String {
    lr_utils::crypto::sha256_hex(url.as_bytes())
}

/// Record that "Configure" wrote `url` into `file` for this client, so
/// later passes recognize the file by that record.
pub fn record_configured(
    state_path: &std::path::Path,
    file: PathBuf,
    client_id: &str,
    template_id: &str,
    url: &str,
    ca_bundle: Option<&str>,
) {
    let _state_lock = STATE_LOCK.lock();
    let mut state = SyncState::load(state_path);
    state
        .seen
        .insert((client_id.to_string(), template_id.to_string()));
    state.files.insert(
        file,
        FileRecord {
            client_id: client_id.to_string(),
            template_id: template_id.to_string(),
            url_sha256: url_hash(url),
            ca_bundle: ca_bundle.map(str::to_string),
        },
    );
    if let Err(e) = state.save(state_path) {
        tracing::warn!("Could not save the proxy sync state: {e}");
    }
}

/// Record that the proxy configuration was removed from `file`.
pub fn record_unconfigured(state_path: &std::path::Path, file: &std::path::Path) {
    let _state_lock = STATE_LOCK.lock();
    let mut state = SyncState::load(state_path);
    if state.files.remove(file).is_some() {
        if let Err(e) = state.save(state_path) {
            tracing::warn!("Could not save the proxy sync state: {e}");
        }
    }
}

/// Reconciles tool configs, with its state on disk.
pub struct Reconciler {
    /// Where the previous content of a rewritten tool config is kept.
    backup_dir: PathBuf,
    /// Where the state is persisted; `None` keeps it in memory only.
    state_path: Option<PathBuf>,
    /// The state when there is no file to keep it in.
    memory: SyncState,
}

impl Default for Reconciler {
    fn default() -> Self {
        Self::new(super::backup::default_backup_dir(), default_state_path())
    }
}

impl Reconciler {
    /// A reconciler that keeps its backups in `backup_dir` and its state in
    /// `state_path` (tests use temp paths so nothing lands in, or is pruned
    /// from, the user's real directories).
    pub fn new(backup_dir: PathBuf, state_path: Option<PathBuf>) -> Self {
        Self {
            backup_dir,
            state_path,
            memory: SyncState::default(),
        }
    }

    /// Bring every tool config that mentions a known client in line with
    /// `clients`.
    ///
    /// `url_for` yields a client's current proxy URL (`None` while the
    /// listener is down); `plan_for` builds the plan for a template, with a
    /// URL to apply or without one to locate the file and undo.
    pub fn reconcile(
        &mut self,
        clients: &[lr_config::Client],
        ca_cert_path: &str,
        url_for: &dyn Fn(&str) -> Option<String>,
        plan_for: &dyn Fn(&str, Option<&str>) -> ProxyPlan,
    ) -> Vec<SyncOutcome> {
        let _state_lock = STATE_LOCK.lock();
        let mut state = match &self.state_path {
            Some(path) => SyncState::load(path),
            None => self.memory.clone(),
        };
        let before = state.clone();
        let outcomes = self.reconcile_state(&mut state, clients, ca_cert_path, url_for, plan_for);
        if let Some(path) = self.state_path.as_ref().filter(|_| state != before) {
            if let Err(e) = state.save(path) {
                tracing::warn!("Could not save the proxy sync state: {e}");
            }
        }
        self.memory = state;
        outcomes
    }

    fn reconcile_state(
        &self,
        state: &mut SyncState,
        clients: &[lr_config::Client],
        ca_cert_path: &str,
        url_for: &dyn Fn(&str) -> Option<String>,
        plan_for: &dyn Fn(&str, Option<&str>) -> ProxyPlan,
    ) -> Vec<SyncOutcome> {
        for client in clients {
            if let Some(template) = client.template_id.as_ref() {
                state.seen.insert((client.id.clone(), template.clone()));
            }
        }
        let existing_ids: Vec<String> = clients.iter().map(|c| c.id.clone()).collect();
        let wants_proxy = |client_id: &str, template_id: &str| {
            clients.iter().any(|c| {
                c.id == client_id
                    && c.template_id.as_deref() == Some(template_id)
                    && c.llm_proxy_enabled()
            })
        };

        // Every pair that may have written a file, from both sources.
        let mut pairs: BTreeSet<(String, String)> = state.seen.clone();
        for record in state.files.values() {
            pairs.insert((record.client_id.clone(), record.template_id.clone()));
        }
        let mut by_file: BTreeMap<PathBuf, Vec<Candidate>> = BTreeMap::new();
        for (client_id, template_id) in &pairs {
            let plan = plan_for(template_id, None);
            if !plan.supports_undo() {
                continue;
            }
            let Some(file) = plan.file else {
                continue;
            };
            by_file.entry(file).or_default().push(Candidate {
                client_id: client_id.clone(),
                template_id: template_id.clone(),
                wants_proxy: wants_proxy(client_id, template_id),
            });
        }
        // Records of files no pair maps to any more (a template whose plan
        // moved) cannot be acted on.
        state.files.retain(|file, _| by_file.contains_key(file));

        let mut outcomes = Vec::new();
        let mut failed_files = BTreeSet::new();
        for (file, candidates) in &by_file {
            // Read and rewrite under the shared lock, so an integration sync
            // editing the same file cannot slip in between.
            let _guard = super::EXTERNAL_CONFIG_LOCK.lock();
            let Ok(content) = std::fs::read_to_string(file) else {
                // Not there (the usual case) or unreadable: nothing to keep.
                state.files.remove(file);
                continue;
            };

            // What LocalRouter recorded writing here, if the file still
            // carries it (the user may have replaced it by hand since).
            let recorded = state
                .files
                .get(file)
                .filter(|r| content.contains(&url_marker(&r.client_id)))
                .and_then(|r| {
                    candidates
                        .iter()
                        .position(|c| c.client_id == r.client_id && c.template_id == r.template_id)
                });
            if recorded.is_none() {
                state.files.remove(file);
            }
            let verdict = match recorded {
                Some(i) if candidates[i].wants_proxy => Verdict::Owned(i),
                Some(i) => Verdict::Stale(i),
                None => classify(&content, ca_cert_path, candidates, &existing_ids),
            };

            match verdict {
                Verdict::Untouched => {}
                Verdict::Owned(i) => {
                    let owner = &candidates[i];
                    let Some(url) = url_for(&owner.client_id) else {
                        continue; // listener down: nothing better to write
                    };
                    let hash = url_hash(&url);
                    let plan = plan_for(&owner.template_id, Some(&url));
                    let up_to_date = match state.files.get(file) {
                        Some(record) => {
                            record.url_sha256 == hash && record.ca_bundle == plan.ca_bundle
                        }
                        None => {
                            !needs_refresh(&content, Some(&url))
                                && plan.ca_bundle.as_ref().is_none_or(|b| content.contains(b))
                        }
                    };
                    let record = FileRecord {
                        client_id: owner.client_id.clone(),
                        template_id: owner.template_id.clone(),
                        url_sha256: hash,
                        ca_bundle: plan.ca_bundle.clone(),
                    };
                    if up_to_date {
                        state.files.insert(file.clone(), record);
                        continue;
                    }
                    match proxy_setup::apply_in(&plan, &self.backup_dir) {
                        Ok(_) => {
                            state.files.insert(file.clone(), record);
                            outcomes.push(SyncOutcome::Refreshed {
                                file: file.clone(),
                                client_id: owner.client_id.clone(),
                            });
                        }
                        Err(error) => {
                            failed_files.insert(file.clone());
                            outcomes.push(SyncOutcome::Failed {
                                file: file.clone(),
                                error,
                            });
                        }
                    }
                }
                verdict @ (Verdict::Stale(_) | Verdict::Orphaned) => {
                    // Every candidate of this file undoes the same keys.
                    let template = match verdict {
                        Verdict::Stale(i) => &candidates[i].template_id,
                        _ => &candidates[0].template_id,
                    };
                    let plan = plan_for(template, None);
                    match proxy_setup::unapply_in(&plan, &self.backup_dir) {
                        Ok(result) => {
                            state.files.remove(file);
                            if !result.modified_files.is_empty() {
                                outcomes.push(SyncOutcome::Removed { file: file.clone() });
                            }
                        }
                        Err(error) => {
                            failed_files.insert(file.clone());
                            outcomes.push(SyncOutcome::Failed {
                                file: file.clone(),
                                error,
                            });
                        }
                    }
                }
            }
        }

        // A deleted client's pair is only needed until its tool config has
        // been dealt with; keep it while that failed, so it is retried.
        let pending_files: BTreeSet<&PathBuf> = failed_files.iter().collect();
        state.seen.retain(|(client_id, template_id)| {
            existing_ids.contains(client_id)
                || plan_for(template_id, None)
                    .file
                    .is_some_and(|file| pending_files.contains(&file))
        });
        outcomes
    }
}

/// Quiet period after a trigger before acting, so a burst of events (a client
/// edit emits several) results in one pass.
const DEBOUNCE: Duration = Duration::from_millis(500);

#[derive(Default)]
struct Pending {
    /// Clients changed: reconcile tool configs.
    clients: AtomicBool,
    /// App config changed: check the listener still matches `proxy.host/port`.
    config: AtomicBool,
    /// Server restart requested: rebind the listener unconditionally.
    restart: AtomicBool,
    wake: tokio::sync::Notify,
}

/// Handle to the background task that keeps the proxy listener bound where
/// the config says and the tool configs pointing at it.
#[derive(Clone)]
pub struct ProxyClientSync {
    pending: Arc<Pending>,
}

impl ProxyClientSync {
    /// Start the task. Runs one reconcile pass straight away (the port or a
    /// client's mode may have changed while the app was not running).
    pub fn spawn(
        proxy: Arc<ProxyService>,
        config_manager: lr_config::ConfigManager,
        client_manager: Arc<lr_clients::ClientManager>,
    ) -> Self {
        let pending = Arc::new(Pending::default());
        pending.clients.store(true, Ordering::SeqCst);
        let task_pending = pending.clone();
        tokio::spawn(async move {
            let mut reconciler = Reconciler::default();
            let mut first = true;
            loop {
                if !first {
                    task_pending.wake.notified().await;
                    tokio::time::sleep(DEBOUNCE).await;
                }
                first = false;

                let restart = task_pending.restart.swap(false, Ordering::SeqCst);
                let config_changed = task_pending.config.swap(false, Ordering::SeqCst);
                let mut reconcile = task_pending.clients.swap(false, Ordering::SeqCst);

                let config = config_manager.get();
                if restart || config_changed {
                    proxy.apply_config(&config);
                }
                let (host, port) = (config.proxy.host.as_str(), config.proxy.port);
                if restart {
                    // A restart must not leave the old listener behind, and
                    // may land on a different port.
                    if let Err(e) = proxy.restart(host, port).await {
                        tracing::warn!("Failed to restart the inspection proxy: {e}");
                    }
                    reconcile = true;
                } else if config_changed {
                    match proxy.ensure(host, port).await {
                        Ok(rebound) => reconcile |= rebound,
                        Err(e) => {
                            tracing::warn!("Failed to rebind the inspection proxy: {e}");
                        }
                    }
                }

                if reconcile {
                    let ca_cert_path = proxy.ca_cert_path().display().to_string();
                    let outcomes = reconciler.reconcile(
                        &config.clients,
                        &ca_cert_path,
                        &|client_id| {
                            let secret = client_manager.get_secret(client_id).ok().flatten()?;
                            proxy.client_proxy_url(client_id, &secret)
                        },
                        &|template_id, url| production_plan(template_id, url, &ca_cert_path),
                    );
                    for outcome in outcomes {
                        // Never log the URL: it embeds the client secret.
                        match outcome {
                            SyncOutcome::Refreshed { file, client_id } => tracing::info!(
                                "Updated the proxy URL for client {client_id} in {}",
                                file.display()
                            ),
                            SyncOutcome::Removed { file } => tracing::info!(
                                "Removed stale proxy configuration from {}",
                                file.display()
                            ),
                            SyncOutcome::Failed { file, error } => tracing::warn!(
                                "Could not update proxy configuration in {}: {error}",
                                file.display()
                            ),
                        }
                    }
                }
            }
        });
        Self { pending }
    }

    /// Clients changed (mode, template, secret, deletion): reconcile configs.
    pub fn clients_changed(&self) {
        self.pending.clients.store(true, Ordering::SeqCst);
        self.pending.wake.notify_one();
    }

    /// The app config changed: rebind if `proxy.host` / `proxy.port` moved.
    pub fn config_changed(&self) {
        self.pending.config.store(true, Ordering::SeqCst);
        self.pending.wake.notify_one();
    }

    /// The server is being restarted: restart the proxy listener with it.
    pub fn restart_requested(&self) {
        self.pending.restart.store(true, Ordering::SeqCst);
        self.pending.wake.notify_one();
    }
}

/// The plan as the "Configure" button builds it, including the combined CA
/// bundle, which has to exist before a config pointing at it is written.
pub fn production_plan(
    template_id: &str,
    proxy_url: Option<&str>,
    ca_cert_path: &str,
) -> ProxyPlan {
    let mut plan = proxy_setup::plan_for(Some(template_id), proxy_url, ca_cert_path);
    if plan.ca_bundle.is_some() && proxy_url.is_some() {
        if let Err(e) = super::ca_bundle::ensure_combined_bundle(std::path::Path::new(ca_cert_path))
        {
            tracing::warn!("Could not prepare the combined CA bundle for {template_id}: {e}");
            plan.disable_auto();
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_config::LlmMode;
    use std::path::Path;

    const CA: &str = "/ca/root-ca.pem";

    fn cand(id: &str, wants_proxy: bool) -> Candidate {
        Candidate {
            client_id: id.to_string(),
            template_id: "codex".to_string(),
            wants_proxy,
        }
    }

    fn url(id: &str, port: u16) -> String {
        format!("http://{id}:lr-secret@127.0.0.1:{port}")
    }

    fn ids(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_file_without_our_config_is_untouched() {
        let content = "HTTPS_PROXY=http://corp-proxy:8080\nFOO=bar\n";
        assert_eq!(
            classify(
                content,
                CA,
                &[cand("a", true), cand("b", false)],
                &ids(&["a", "b"])
            ),
            Verdict::Untouched
        );
    }

    #[test]
    fn config_of_a_proxy_mode_client_is_owned() {
        let content = format!("HTTPS_PROXY={}\nSSL_CERT_FILE={CA}\n", url("a", 3626));
        assert_eq!(
            classify(
                &content,
                CA,
                &[cand("b", true), cand("a", true)],
                &ids(&["a", "b"])
            ),
            Verdict::Owned(1)
        );
    }

    #[test]
    fn config_of_a_client_that_left_proxy_mode_or_was_deleted_is_stale() {
        let content = format!("HTTPS_PROXY={}\nSSL_CERT_FILE={CA}\n", url("a", 3626));
        // Still exists, no longer proxying.
        assert_eq!(
            classify(&content, CA, &[cand("a", false)], &ids(&["a"])),
            Verdict::Stale(0)
        );
        // Deleted: remembered as a candidate, absent from the existing ids.
        assert_eq!(
            classify(&content, CA, &[cand("a", false)], &[]),
            Verdict::Stale(0)
        );
    }

    #[test]
    fn another_proxy_client_of_the_same_tool_does_not_save_a_stale_config() {
        // The file is configured for A (which left proxy mode); B proxies
        // through the same tool but was never written to this file. A's
        // entry is dead either way and must go.
        let content = format!("HTTPS_PROXY={}\n", url("a", 3626));
        assert_eq!(
            classify(
                &content,
                CA,
                &[cand("a", false), cand("b", true)],
                &ids(&["a", "b"])
            ),
            Verdict::Stale(0)
        );
    }

    #[test]
    fn the_owner_wins_over_a_stale_marker_in_the_same_file() {
        let content = format!("A={}\nB={}\n", url("a", 3626), url("b", 3626));
        assert_eq!(
            classify(
                &content,
                CA,
                &[cand("a", false), cand("b", true)],
                &ids(&["a", "b"])
            ),
            Verdict::Owned(1)
        );
    }

    #[test]
    fn ca_only_config_is_orphaned_once_no_client_proxies_through_the_tool() {
        let content = format!("GOOSE_CA_CERT_PATH: {CA}\n");
        // A proxy-mode client still uses the tool: keep.
        assert_eq!(
            classify(&content, CA, &[cand("a", true)], &ids(&["a"])),
            Verdict::Untouched
        );
        // Nobody does any more: remove.
        assert_eq!(
            classify(&content, CA, &[cand("a", false)], &ids(&["a"])),
            Verdict::Orphaned
        );
    }

    #[test]
    fn hand_written_config_for_another_existing_client_is_not_an_orphan() {
        // Set up by hand for a client with a different (or no) template: it
        // names our CA and no candidate proxies, but it is somebody's config.
        let content = format!("HTTPS_PROXY={}\nSSL_CERT_FILE={CA}\n", url("custom", 3626));
        assert_eq!(
            classify(&content, CA, &[cand("a", false)], &ids(&["a", "custom"])),
            Verdict::Untouched
        );
        // The same file once that client is gone is a leftover.
        assert_eq!(
            classify(&content, CA, &[cand("a", false)], &ids(&["a"])),
            Verdict::Orphaned
        );
    }

    #[test]
    fn an_unknown_ca_path_never_makes_a_file_orphaned() {
        assert_eq!(
            classify("SSL_CERT_FILE=\n", "", &[cand("a", false)], &ids(&["a"])),
            Verdict::Untouched
        );
    }

    #[test]
    fn refresh_only_when_running_and_the_url_differs() {
        let content = format!("HTTPS_PROXY={}\n", url("a", 3626));
        assert!(!needs_refresh(&content, Some(&url("a", 3626))));
        assert!(needs_refresh(&content, Some(&url("a", 4000))));
        // Listener down: there is nothing better to write, so leave it.
        assert!(!needs_refresh(&content, None));
    }

    // --- Reconcile against real files in a temp dir -------------------------

    fn client(id: &str, template: &str, mode: LlmMode) -> lr_config::Client {
        let mut c = lr_config::Client::new_with_strategy(id.to_string(), "strategy".to_string());
        c.id = id.to_string();
        c.template_id = Some(template.to_string());
        c.llm_mode = mode;
        c
    }

    /// The real plan for a template, with its target redirected into `dir`
    /// so no test ever reads or writes a real tool config.
    fn plan_in<'a>(dir: &'a Path) -> impl Fn(&str, Option<&str>) -> ProxyPlan + 'a {
        move |template_id, proxy_url| {
            let mut plan = proxy_setup::plan_for(Some(template_id), proxy_url, CA);
            if plan.file.is_some() {
                plan.file = Some(dir.join(format!("{template_id}.conf")));
            }
            plan
        }
    }

    /// Backups go under the test's temp dir, never the user's backup dir.
    fn backups(dir: &Path) -> PathBuf {
        dir.join("backups")
    }

    /// State persisted under the test's temp dir: a new reconciler on the
    /// same dir is the app after a restart.
    fn state_path(dir: &Path) -> PathBuf {
        dir.join("proxy-client-sync.json")
    }

    fn reconciler_in(dir: &Path) -> Reconciler {
        Reconciler::new(backups(dir), Some(state_path(dir)))
    }

    /// "Configure" as the command does it: apply, then record.
    fn configure_for_test(dir: &Path, template: &str, client_id: &str, url: &str) {
        let plan = plan_in(dir)(template, Some(url));
        apply_for_test(dir, &plan).unwrap();
        record_configured(
            &state_path(dir),
            plan.file.unwrap(),
            client_id,
            template,
            url,
            plan.ca_bundle.as_deref(),
        );
    }

    /// What clicking "Configure" does, with the backup redirected.
    fn apply_for_test(
        dir: &Path,
        plan: &ProxyPlan,
    ) -> Result<crate::ui::commands_clients::LaunchResult, String> {
        proxy_setup::apply_in(plan, &backups(dir))
    }

    fn read(dir: &Path, template: &str) -> String {
        std::fs::read_to_string(dir.join(format!("{template}.conf"))).unwrap()
    }

    #[test]
    fn a_port_change_rewrites_the_url_of_a_configured_client_only() {
        let dir = tempfile::tempdir().unwrap();
        let plan_for = plan_in(dir.path());
        let clients = vec![
            client("configured", "codex", LlmMode::Proxy),
            // In proxy mode too, but the user never clicked Configure for it.
            client("never-configured", "claude-code", LlmMode::Proxy),
        ];

        // The user configures Codex while the proxy is on port 3626.
        apply_for_test(
            dir.path(),
            &plan_for("codex", Some(&url("configured", 3626))),
        )
        .unwrap();

        let mut reconciler = reconciler_in(dir.path());
        let on_port = |port: u16| move |id: &str| Some(url(id, port));

        // Same port: nothing to do, and nothing is created for the other tool.
        assert!(reconciler
            .reconcile(&clients, CA, &on_port(3626), &plan_for)
            .is_empty());
        assert!(!dir.path().join("claude-code.conf").exists());

        // The proxy moves: the configured tool follows.
        let outcomes = reconciler.reconcile(&clients, CA, &on_port(4000), &plan_for);
        assert_eq!(
            outcomes,
            vec![SyncOutcome::Refreshed {
                file: dir.path().join("codex.conf"),
                client_id: "configured".into()
            }]
        );
        let content = read(dir.path(), "codex");
        assert!(content.contains(&url("configured", 4000)));
        assert!(!content.contains(":3626"));
        assert!(!dir.path().join("claude-code.conf").exists());

        // And it is stable afterwards.
        assert!(reconciler
            .reconcile(&clients, CA, &on_port(4000), &plan_for)
            .is_empty());
    }

    #[test]
    fn a_stopped_proxy_leaves_configured_tools_alone() {
        let dir = tempfile::tempdir().unwrap();
        let plan_for = plan_in(dir.path());
        let clients = vec![client("a", "codex", LlmMode::Proxy)];
        apply_for_test(dir.path(), &plan_for("codex", Some(&url("a", 3626)))).unwrap();
        let before = read(dir.path(), "codex");

        let outcomes = reconciler_in(dir.path()).reconcile(&clients, CA, &|_| None, &plan_for);
        assert!(outcomes.is_empty());
        assert_eq!(read(dir.path(), "codex"), before);
    }

    #[test]
    fn leaving_proxy_mode_removes_the_config_and_keeps_the_users_own_lines() {
        let dir = tempfile::tempdir().unwrap();
        let plan_for = plan_in(dir.path());
        std::fs::write(dir.path().join("codex.conf"), "OPENAI_API_KEY=keep-me\n").unwrap();
        apply_for_test(dir.path(), &plan_for("codex", Some(&url("a", 3626)))).unwrap();
        assert!(read(dir.path(), "codex").contains("HTTPS_PROXY"));

        let mut reconciler = reconciler_in(dir.path());
        let clients = vec![client("a", "codex", LlmMode::Gateway)];
        let outcomes = reconciler.reconcile(&clients, CA, &|id| Some(url(id, 3626)), &plan_for);
        assert_eq!(
            outcomes,
            vec![SyncOutcome::Removed {
                file: dir.path().join("codex.conf")
            }]
        );
        let content = read(dir.path(), "codex");
        assert!(!content.contains("HTTPS_PROXY"));
        assert!(!content.contains(CA));
        assert!(content.contains("OPENAI_API_KEY=keep-me"));

        // Nothing left to do on the next pass.
        assert!(reconciler
            .reconcile(&clients, CA, &|id| Some(url(id, 3626)), &plan_for)
            .is_empty());
    }

    #[test]
    fn deleting_a_client_removes_its_config_on_the_next_pass() {
        let dir = tempfile::tempdir().unwrap();
        let plan_for = plan_in(dir.path());
        apply_for_test(dir.path(), &plan_for("claude-code", Some(&url("a", 3626)))).unwrap();

        let mut reconciler = reconciler_in(dir.path());
        let with_client = vec![client("a", "claude-code", LlmMode::Proxy)];
        assert!(reconciler
            .reconcile(&with_client, CA, &|id| Some(url(id, 3626)), &plan_for)
            .is_empty());

        // The client is deleted; the reconciler still remembers where it was.
        let outcomes = reconciler.reconcile(&[], CA, &|id| Some(url(id, 3626)), &plan_for);
        assert_eq!(
            outcomes,
            vec![SyncOutcome::Removed {
                file: dir.path().join("claude-code.conf")
            }]
        );
        let content = read(dir.path(), "claude-code");
        assert!(!content.contains("HTTPS_PROXY"));
        assert!(!content.contains("NODE_EXTRA_CA_CERTS"));
    }

    #[test]
    fn changing_a_clients_template_removes_the_old_tools_config() {
        let dir = tempfile::tempdir().unwrap();
        let plan_for = plan_in(dir.path());
        apply_for_test(dir.path(), &plan_for("codex", Some(&url("a", 3626)))).unwrap();

        let mut reconciler = reconciler_in(dir.path());
        reconciler.reconcile(
            &[client("a", "codex", LlmMode::Proxy)],
            CA,
            &|id| Some(url(id, 3626)),
            &plan_for,
        );
        let outcomes = reconciler.reconcile(
            &[client("a", "claude-code", LlmMode::Proxy)],
            CA,
            &|id| Some(url(id, 3626)),
            &plan_for,
        );
        assert_eq!(
            outcomes,
            vec![SyncOutcome::Removed {
                file: dir.path().join("codex.conf")
            }]
        );
        // The new tool is not configured on the user's behalf.
        assert!(!dir.path().join("claude-code.conf").exists());
    }

    #[test]
    fn another_clients_config_in_the_same_tool_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let plan_for = plan_in(dir.path());
        // Two Codex clients; the tool is configured for B.
        apply_for_test(dir.path(), &plan_for("codex", Some(&url("b", 3626)))).unwrap();
        let before = read(dir.path(), "codex");

        let clients = vec![
            client("a", "codex", LlmMode::Gateway), // A leaves proxy mode
            client("b", "codex", LlmMode::Proxy),
        ];
        let outcomes =
            reconciler_in(dir.path()).reconcile(&clients, CA, &|id| Some(url(id, 3626)), &plan_for);
        assert!(outcomes.is_empty());
        assert_eq!(read(dir.path(), "codex"), before);
    }

    #[test]
    fn a_rotated_secret_is_written_through() {
        let dir = tempfile::tempdir().unwrap();
        let plan_for = plan_in(dir.path());
        let clients = vec![client("a", "claude-code", LlmMode::Proxy)];
        apply_for_test(dir.path(), &plan_for("claude-code", Some(&url("a", 3626)))).unwrap();

        let rotated = "http://a:lr-rotated@127.0.0.1:3626".to_string();
        let outcomes = reconciler_in(dir.path()).reconcile(
            &clients,
            CA,
            &|_| Some(rotated.clone()),
            &plan_for,
        );
        assert_eq!(outcomes.len(), 1);
        let content = read(dir.path(), "claude-code");
        assert!(content.contains("lr-rotated"));
        assert!(!content.contains("lr-secret"));
    }

    /// Codex configs written before `SSL_CERT_FILE` pointed at the combined
    /// bundle name the bare root CA, which breaks TLS for every command Codex
    /// runs. They are rewritten to the bundle once, recorded or not.
    #[test]
    fn a_codex_config_naming_the_bare_root_ca_moves_to_the_bundle() {
        for recorded in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let plan_for = plan_in(dir.path());
            let clients = vec![client("a", "codex", LlmMode::Proxy)];
            let file = dir.path().join("codex.conf");
            std::fs::write(
                &file,
                format!(
                    "OTHER=1\nHTTPS_PROXY={}\nSSL_CERT_FILE={CA}\n",
                    url("a", 3626)
                ),
            )
            .unwrap();
            if recorded {
                // A record from that version: no bundle in it.
                record_configured(
                    &state_path(dir.path()),
                    file.clone(),
                    "a",
                    "codex",
                    &url("a", 3626),
                    None,
                );
            }

            let mut reconciler = reconciler_in(dir.path());
            let on_port = |id: &str| Some(url(id, 3626));
            let outcomes = reconciler.reconcile(&clients, CA, &on_port, &plan_for);
            assert_eq!(outcomes.len(), 1, "recorded={recorded}: {outcomes:?}");
            let content = read(dir.path(), "codex");
            let bundle = plan_for("codex", None).ca_bundle.unwrap();
            assert!(
                content.contains(&format!("SSL_CERT_FILE={bundle}")),
                "{content}"
            );
            assert!(!content.contains(&format!("SSL_CERT_FILE={CA}")));
            assert!(content.contains("OTHER=1"));

            // Stable afterwards.
            assert!(reconciler
                .reconcile(&clients, CA, &on_port, &plan_for)
                .is_empty());
        }
    }

    #[test]
    fn a_malformed_tool_config_is_reported_and_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let plan_for = plan_in(dir.path());
        let clients = vec![client("a", "claude-code", LlmMode::Proxy)];
        // Mentions the client's URL but is not valid JSON.
        let broken = format!("{{ \"env\": {{ \"HTTPS_PROXY\": \"{}\" ", url("a", 3626));
        std::fs::write(dir.path().join("claude-code.conf"), &broken).unwrap();

        let outcomes =
            reconciler_in(dir.path()).reconcile(&clients, CA, &|id| Some(url(id, 4000)), &plan_for);
        assert!(matches!(outcomes.as_slice(), [SyncOutcome::Failed { .. }]));
        assert_eq!(read(dir.path(), "claude-code"), broken);
    }

    #[test]
    fn every_undoable_template_cleans_up_after_a_deleted_client() {
        // Whatever shape a template's config takes, a deleted client's proxy
        // URL must not survive in it.
        for template in [
            "claude-code",
            "codex",
            "openclaw",
            "opencode",
            "zed",
            "cline",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let plan_for = plan_in(dir.path());
            apply_for_test(dir.path(), &plan_for(template, Some(&url("gone", 3626)))).unwrap();
            assert!(
                read(dir.path(), template).contains("://gone:"),
                "'{template}' config does not carry the client marker"
            );

            let mut reconciler = reconciler_in(dir.path());
            reconciler.reconcile(
                &[client("gone", template, LlmMode::Proxy)],
                CA,
                &|id| Some(url(id, 3626)),
                &plan_for,
            );
            reconciler.reconcile(&[], CA, &|_| None, &plan_for);

            let file = dir.path().join(format!("{template}.conf"));
            let left = std::fs::read_to_string(&file).unwrap_or_default();
            assert!(
                !left.contains("://gone:"),
                "'{template}' kept a deleted client's proxy URL"
            );
        }
    }

    #[test]
    fn a_client_deleted_before_a_restart_is_cleaned_up_on_the_next_start() {
        let dir = tempfile::tempdir().unwrap();
        let plan_for = plan_in(dir.path());
        // Configured, then deleted and the app quit before any pass ran.
        configure_for_test(dir.path(), "codex", "gone", &url("gone", 3626));

        // The next start: a new reconciler that never saw the client.
        let outcomes =
            reconciler_in(dir.path()).reconcile(&[], CA, &|id| Some(url(id, 3626)), &plan_for);
        assert_eq!(
            outcomes,
            vec![SyncOutcome::Removed {
                file: dir.path().join("codex.conf")
            }]
        );
        assert!(!read(dir.path(), "codex").contains("://gone:"));
        // Nothing of the deleted client is kept once it is dealt with.
        assert_eq!(
            SyncState::load(&state_path(dir.path())),
            SyncState::default()
        );
    }

    #[test]
    fn a_pair_seen_in_an_earlier_session_is_remembered() {
        let dir = tempfile::tempdir().unwrap();
        let plan_for = plan_in(dir.path());
        // Configured by hand (no record), seen by a pass, then the app quits.
        apply_for_test(dir.path(), &plan_for("claude-code", Some(&url("a", 3626)))).unwrap();
        let clients = vec![client("a", "claude-code", LlmMode::Proxy)];
        assert!(reconciler_in(dir.path())
            .reconcile(&clients, CA, &|id| Some(url(id, 3626)), &plan_for)
            .is_empty());

        // Deleted while the app was not running.
        let outcomes =
            reconciler_in(dir.path()).reconcile(&[], CA, &|id| Some(url(id, 3626)), &plan_for);
        assert_eq!(outcomes.len(), 1);
        assert!(!read(dir.path(), "claude-code").contains("://a:"));
    }

    #[test]
    fn the_state_file_never_holds_a_secret() {
        let dir = tempfile::tempdir().unwrap();
        configure_for_test(dir.path(), "codex", "a", &url("a", 3626));
        let clients = vec![client("a", "codex", LlmMode::Proxy)];
        reconciler_in(dir.path()).reconcile(
            &clients,
            CA,
            &|id| Some(url(id, 4000)),
            &plan_in(dir.path()),
        );
        let state = std::fs::read_to_string(state_path(dir.path())).unwrap();
        assert!(!state.contains("lr-secret"), "{state}");
        assert!(!state.contains("127.0.0.1"), "{state}");
        let record =
            &SyncState::load(&state_path(dir.path())).files[&dir.path().join("codex.conf")];
        assert_eq!(record.client_id, "a");
        assert_eq!(record.url_sha256, url_hash(&url("a", 4000)));
    }

    #[test]
    fn a_recorded_url_is_not_rewritten_on_every_pass() {
        let dir = tempfile::tempdir().unwrap();
        let plan_for = plan_in(dir.path());
        let clients = vec![client("a", "codex", LlmMode::Proxy)];
        configure_for_test(dir.path(), "codex", "a", &url("a", 3626));
        // The file holds the URL in a form a substring check does not find
        // (here: percent-encoded), but the client marker is intact.
        let path = dir.path().join("codex.conf");
        let content = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, content.replace("lr-secret", "lr%2Dsecret")).unwrap();
        let before = std::fs::read_to_string(&path).unwrap();

        let outcomes =
            reconciler_in(dir.path()).reconcile(&clients, CA, &|id| Some(url(id, 3626)), &plan_for);
        assert!(outcomes.is_empty(), "{outcomes:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn a_file_the_user_repointed_is_no_longer_ours() {
        let dir = tempfile::tempdir().unwrap();
        let plan_for = plan_in(dir.path());
        configure_for_test(dir.path(), "codex", "a", &url("a", 3626));
        // The user replaced our proxy with their own (and the CA line).
        std::fs::write(
            dir.path().join("codex.conf"),
            "HTTPS_PROXY=http://corp-proxy:8080\n",
        )
        .unwrap();
        let clients = vec![client("a", "codex", LlmMode::Gateway)];
        let outcomes =
            reconciler_in(dir.path()).reconcile(&clients, CA, &|id| Some(url(id, 3626)), &plan_for);
        assert!(outcomes.is_empty(), "{outcomes:?}");
        assert_eq!(
            read(dir.path(), "codex"),
            "HTTPS_PROXY=http://corp-proxy:8080\n"
        );
        assert!(SyncState::load(&state_path(dir.path())).files.is_empty());
    }

    #[test]
    fn unconfiguring_forgets_the_record() {
        let dir = tempfile::tempdir().unwrap();
        configure_for_test(dir.path(), "codex", "a", &url("a", 3626));
        let file = dir.path().join("codex.conf");
        assert!(SyncState::load(&state_path(dir.path()))
            .files
            .contains_key(&file));
        record_unconfigured(&state_path(dir.path()), &file);
        assert!(SyncState::load(&state_path(dir.path())).files.is_empty());
    }
}
