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
//! This module reconciles that from the files themselves. A LocalRouter proxy
//! URL is `http://<client_id>:<secret>@host:port`, so a tool's config says
//! which client it was configured for; no separate "was applied" flag exists
//! that could drift from what is on disk. It never configures a tool that was
//! not already configured for that client.
//!
//! Only the proxy URL and the path of the public root CA certificate are ever
//! written to a tool's config.

use super::proxy::ProxyService;
use super::proxy_setup::{self, ProxyPlan};
use std::collections::{BTreeMap, BTreeSet, HashMap};
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
    let names_our_ca = !ca_cert_path.is_empty() && content.contains(ca_cert_path);
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

/// Reconcile state kept across passes.
pub struct Reconciler {
    /// Where the previous content of a rewritten tool config is kept.
    backup_dir: PathBuf,
    /// Every (client id, template id) pair seen this session. Deleted clients
    /// and changed templates stay in here so their config can still be found
    /// and removed.
    seen: BTreeSet<(String, String)>,
    /// The URL last written to each file, so a file whose format hides the URL
    /// from a plain substring check is not rewritten on every pass.
    last_written: HashMap<PathBuf, String>,
}

impl Default for Reconciler {
    fn default() -> Self {
        Self::with_backup_dir(super::backup::default_backup_dir())
    }
}

impl Reconciler {
    /// A reconciler that keeps its backups in `backup_dir` (tests use a temp
    /// dir so nothing lands in, or is pruned from, the user's real one).
    pub fn with_backup_dir(backup_dir: PathBuf) -> Self {
        Self {
            backup_dir,
            seen: BTreeSet::new(),
            last_written: HashMap::new(),
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
        for client in clients {
            if let Some(template) = client.template_id.as_ref() {
                self.seen.insert((client.id.clone(), template.clone()));
            }
        }
        let existing_ids: Vec<String> = clients.iter().map(|c| c.id.clone()).collect();

        let mut by_file: BTreeMap<PathBuf, Vec<Candidate>> = BTreeMap::new();
        for (client_id, template_id) in &self.seen {
            let plan = plan_for(template_id, None);
            if !plan.supports_undo() {
                continue;
            }
            let Some(file) = plan.file else {
                continue;
            };
            let wants_proxy = clients.iter().any(|c| {
                c.id == *client_id
                    && c.template_id.as_deref() == Some(template_id.as_str())
                    && c.llm_proxy_enabled()
            });
            by_file.entry(file).or_default().push(Candidate {
                client_id: client_id.clone(),
                template_id: template_id.clone(),
                wants_proxy,
            });
        }

        let mut outcomes = Vec::new();
        for (file, candidates) in by_file {
            // Read and rewrite under the shared lock, so an integration sync
            // editing the same file cannot slip in between.
            let _guard = super::EXTERNAL_CONFIG_LOCK.lock();
            let Ok(content) = std::fs::read_to_string(&file) else {
                continue; // not there (the usual case) or unreadable: nothing to maintain
            };
            match classify(&content, ca_cert_path, &candidates, &existing_ids) {
                Verdict::Untouched => {}
                Verdict::Owned(i) => {
                    let owner = &candidates[i];
                    let Some(url) =
                        url_for(&owner.client_id).filter(|url| needs_refresh(&content, Some(url)))
                    else {
                        continue;
                    };
                    if self.last_written.get(&file) == Some(&url) {
                        continue;
                    }
                    let plan = plan_for(&owner.template_id, Some(&url));
                    match proxy_setup::apply_in(&plan, &self.backup_dir) {
                        Ok(_) => {
                            self.last_written.insert(file.clone(), url);
                            outcomes.push(SyncOutcome::Refreshed {
                                file,
                                client_id: owner.client_id.clone(),
                            });
                        }
                        Err(error) => outcomes.push(SyncOutcome::Failed { file, error }),
                    }
                }
                verdict @ (Verdict::Stale(_) | Verdict::Orphaned) => {
                    // Every candidate of this file undoes the same keys.
                    let template = match verdict {
                        Verdict::Stale(i) => &candidates[i].template_id,
                        _ => &candidates[0].template_id,
                    };
                    let plan = plan_for(template, None);
                    self.last_written.remove(&file);
                    match proxy_setup::unapply_in(&plan, &self.backup_dir) {
                        Ok(result) if result.modified_files.is_empty() => {}
                        Ok(_) => outcomes.push(SyncOutcome::Removed { file }),
                        Err(error) => outcomes.push(SyncOutcome::Failed { file, error }),
                    }
                }
            }
        }
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

/// The plan as the "Configure" button builds it, including Aider's combined
/// CA bundle, which has to exist before a config pointing at it is written.
pub fn production_plan(
    template_id: &str,
    proxy_url: Option<&str>,
    ca_cert_path: &str,
) -> ProxyPlan {
    if template_id == "aider" && proxy_url.is_some() {
        if let Err(e) = super::ca_bundle::ensure_combined_bundle(std::path::Path::new(ca_cert_path))
        {
            tracing::warn!("Could not prepare the combined CA bundle for Aider: {e}");
        }
    }
    proxy_setup::plan_for(Some(template_id), proxy_url, ca_cert_path)
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

    fn reconciler_in(dir: &Path) -> Reconciler {
        Reconciler::with_backup_dir(backups(dir))
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
}
