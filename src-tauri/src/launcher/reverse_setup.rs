//! Per-provider relocation plans for reverse-proxy mode.
//!
//! Wrapping a local provider means taking over the address apps already use, so
//! the provider itself has to move first. That move is provider-specific and
//! not always scriptable: Ollama reads an environment variable, LM Studio has a
//! CLI, and the GUI-only apps (Jan, GPT4All) can only be changed by the user.
//!
//! This module encodes what is known per provider behind one shape
//! ([`ReversePlan`]), in the spirit of [`crate::launcher::proxy_setup`]:
//! - `auto` — commands LocalRouter can run itself, with an `undo` counterpart;
//! - `oneoff_command` — a command the user can run to start the provider on the
//!   new port for this session;
//! - `manual_steps` — GUI instructions when nothing else is possible.
//!
//! Everything here that *executes* is deliberately explicit: the plan is built
//! purely (testable), and [`apply`] is the only function that touches the
//! machine.

use std::process::Command;
use std::time::Duration;

use crate::ui::commands_clients::LaunchResult;

/// Default relocation for each supported local provider: the port apps already
/// use, and the port the provider moves to.
///
/// The relocated ports are deliberately adjacent to the originals so the
/// mapping is obvious in logs and config (`11434` → `11435`). llama.cpp gets
/// `8082` rather than `8081` because it shares `8080` with LocalAI and the two
/// may both be present.
pub const DEFAULT_PORTS: &[(&str, u16, u16)] = &[
    ("ollama", 11434, 11435),
    ("lmstudio", 1234, 1235),
    ("jan", 1337, 1338),
    ("gpt4all", 4891, 4892),
    ("localai", 8080, 8081),
    ("llamacpp", 8080, 8082),
];

/// Client-template id → provider key (`reverse-ollama` → `ollama`).
pub fn provider_key_for_template(template_id: &str) -> Option<&'static str> {
    let key = template_id.strip_prefix("reverse-")?;
    DEFAULT_PORTS
        .iter()
        .find(|(k, _, _)| *k == key)
        .map(|(k, _, _)| *k)
}

/// Default (listen_port, upstream_port) for a provider key.
pub fn default_ports(provider_key: &str) -> Option<(u16, u16)> {
    DEFAULT_PORTS
        .iter()
        .find(|(k, _, _)| *k == provider_key)
        .map(|(_, listen, upstream)| (*listen, *upstream))
}

/// The `lr_config::ProviderType` serde name for a provider key — identical
/// here, but named so callers don't rely on the coincidence.
pub fn provider_type_name(provider_key: &str) -> &str {
    provider_key
}

/// One command in a relocation sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cmd {
    pub program: String,
    pub args: Vec<String>,
    /// Failure is expected/harmless (e.g. quitting an app that isn't running).
    pub ignore_failure: bool,
}

impl Cmd {
    fn new(program: &str, args: &[&str]) -> Self {
        Self {
            program: program.to_string(),
            args: args.iter().map(|a| a.to_string()).collect(),
            ignore_failure: false,
        }
    }

    fn optional(program: &str, args: &[&str]) -> Self {
        Self {
            ignore_failure: true,
            ..Self::new(program, args)
        }
    }

    /// Human-readable form, for the UI and for error messages.
    pub fn display(&self) -> String {
        let args = self
            .args
            .iter()
            .map(|a| {
                if a.contains(' ') {
                    format!("'{a}'")
                } else {
                    a.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        if args.is_empty() {
            self.program.clone()
        } else {
            format!("{} {args}", self.program)
        }
    }
}

/// Everything known about relocating one provider off its original port.
///
/// Relocation is split into phases rather than one command list, because the
/// only thing that actually matters is the *outcome*: the old port has to be
/// released and the new one has to answer. Commands lie — Ollama, for one,
/// refuses an AppleScript quit with "User canceled" — so each phase is
/// verified against the ports rather than against exit codes.
#[derive(Debug, Clone, Default)]
pub struct ReversePlan {
    /// Display name of the wrapped provider ("Ollama").
    pub provider_label: String,
    /// Configuration changes made before the restart (e.g. setting the env var
    /// the provider reads at start-up).
    pub configure: Vec<Cmd>,
    /// The same, for undo (e.g. clearing that env var).
    pub unconfigure: Vec<Cmd>,
    /// Ask the provider to stop, politely.
    pub stop: Vec<Cmd>,
    /// Escalation when the polite stop leaves the port held. Only run if the
    /// port is *still* occupied after `stop`, so a well-behaved provider is
    /// never force-terminated.
    pub force_stop: Vec<Cmd>,
    /// Start the provider again (it re-reads its configuration here).
    pub start: Vec<Cmd>,
    /// A command the user can run to start the provider on the new port.
    pub oneoff_command: Option<String>,
    /// GUI steps, when the provider can't be relocated programmatically.
    pub manual_steps: Vec<String>,
    /// Caveats worth reading before applying.
    pub notes: Vec<String>,
    /// What the user must do for the change to take effect.
    pub restart_hint: Option<String>,
}

impl ReversePlan {
    /// Automation is only claimed when we can both change the configuration and
    /// restart the provider — either half alone leaves the user stranded.
    pub fn supports_auto(&self) -> bool {
        !self.start.is_empty() && (!self.configure.is_empty() || !self.stop.is_empty())
    }

    /// Anything we can relocate automatically, we can move back automatically —
    /// even when there is nothing to un-configure. LM Studio takes its port as
    /// a CLI argument, so its undo is "start it on the original port again"
    /// with an empty `unconfigure`; requiring a non-empty one here would have
    /// silently dropped its Undo button.
    pub fn supports_undo(&self) -> bool {
        self.supports_auto()
    }

    /// Every command automatic relocation may run, in order, as display
    /// strings — so the UI can show exactly what is about to happen. Includes
    /// the escalation, marked, because "we might force-quit your app" is
    /// precisely the part a user deserves to see up front.
    pub fn auto_commands(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .configure
            .iter()
            .chain(self.stop.iter())
            .map(Cmd::display)
            .collect();
        for cmd in &self.force_stop {
            out.push(format!(
                "{} (only if the port is still held)",
                cmd.display()
            ));
        }
        out.extend(self.start.iter().map(Cmd::display));
        out
    }
}

/// Build the relocation plan for `provider_key`, moving it from `listen_port`
/// (which LocalRouter takes over) to `upstream_port`.
pub fn plan_for(provider_key: &str, listen_port: u16, upstream_port: u16) -> ReversePlan {
    let host_port = format!("127.0.0.1:{upstream_port}");
    match provider_key {
        "ollama" => ollama_plan(&host_port, listen_port, upstream_port),
        "lmstudio" => lmstudio_plan(upstream_port),
        "jan" => ReversePlan {
            provider_label: "Jan".to_string(),
            manual_steps: vec![
                "Open Jan → Settings → Advanced Settings.".to_string(),
                format!("Under Local API Server, change the port to {upstream_port}."),
                "Stop and start the local API server so the new port takes effect.".to_string(),
            ],
            notes: vec![
                "Jan's server port can only be changed in its interface — LocalRouter can't \
                 relocate it for you."
                    .to_string(),
            ],
            restart_hint: Some(
                "After Jan is listening on the new port, click Start listener.".to_string(),
            ),
            ..Default::default()
        },
        "gpt4all" => ReversePlan {
            provider_label: "GPT4All".to_string(),
            manual_steps: vec![
                "Open GPT4All → Settings → Application.".to_string(),
                format!("Set 'API Server Port' to {upstream_port} and make sure the local API server is enabled."),
                "Restart GPT4All so it binds the new port.".to_string(),
            ],
            notes: vec![
                "GPT4All's API port is a GUI setting — LocalRouter can't relocate it for you."
                    .to_string(),
            ],
            restart_hint: Some(
                "After GPT4All is listening on the new port, click Start listener.".to_string(),
            ),
            ..Default::default()
        },
        "localai" => ReversePlan {
            provider_label: "LocalAI".to_string(),
            oneoff_command: Some(format!("local-ai run --address 127.0.0.1:{upstream_port}")),
            manual_steps: vec![
                format!("Restart LocalAI bound to the new port, e.g. `local-ai run --address 127.0.0.1:{upstream_port}`."),
                format!("If you run LocalAI in Docker, change the published port to {upstream_port} (e.g. `-p {upstream_port}:8080`)."),
            ],
            notes: vec![
                "LocalAI is usually started by hand or by Docker, so LocalRouter doesn't restart \
                 it for you — relaunch it on the new port and then start the listener."
                    .to_string(),
                format!("Port {listen_port} is a common default for other services; make sure LocalAI is what's actually on it."),
            ],
            restart_hint: Some(
                "Relaunch LocalAI on the new port, then click Start listener.".to_string(),
            ),
            ..Default::default()
        },
        "llamacpp" => ReversePlan {
            provider_label: "llama.cpp".to_string(),
            oneoff_command: Some(format!(
                "llama-server --host 127.0.0.1 --port {upstream_port} -m <your-model.gguf>"
            )),
            manual_steps: vec![
                format!("Restart llama-server with `--port {upstream_port}` (keep every other flag as-is)."),
            ],
            notes: vec![
                "llama-server takes its port on the command line, so LocalRouter can't move it \
                 for you — restart it with the new port and then start the listener."
                    .to_string(),
            ],
            restart_hint: Some(
                "Relaunch llama-server on the new port, then click Start listener.".to_string(),
            ),
            ..Default::default()
        },
        other => ReversePlan {
            provider_label: other.to_string(),
            manual_steps: vec![format!(
                "Reconfigure {other} to listen on port {upstream_port} instead of {listen_port}, \
                 then start the listener."
            )],
            ..Default::default()
        },
    }
}

/// Ollama reads `OLLAMA_HOST` at start-up, so relocation means setting that
/// variable where Ollama will see it and restarting the server. How to do that
/// is entirely platform-specific.
///
/// Two things learned the hard way on macOS, both encoded below:
/// - **The GUI app must restart, not just the server.** `launchctl setenv`
///   only seeds *newly launched* processes; the already-running app's
///   environment is fixed, so a server it respawns would inherit the old port.
/// - **Ollama refuses an AppleScript quit** ("User canceled", -128), so the
///   polite quit is best-effort and the escalation is what actually frees the
///   port.
fn ollama_plan(host_port: &str, listen_port: u16, upstream_port: u16) -> ReversePlan {
    let mut plan = ReversePlan {
        provider_label: "Ollama".to_string(),
        oneoff_command: Some(format!("OLLAMA_HOST={host_port} ollama serve")),
        restart_hint: Some("Ollama must be restarted for the new port to take effect.".to_string()),
        ..Default::default()
    };

    if cfg!(target_os = "macos") {
        // `launchctl setenv` is what the Ollama docs prescribe for the macOS
        // app: it seeds the environment of GUI-launched processes, so the
        // relaunched app picks the port up. It does not need sudo.
        plan.configure = vec![Cmd::new("launchctl", &["setenv", "OLLAMA_HOST", host_port])];
        plan.unconfigure = vec![Cmd::new("launchctl", &["unsetenv", "OLLAMA_HOST"])];
        plan.stop = vec![Cmd::optional(
            "osascript",
            &["-e", "tell application \"Ollama\" to quit"],
        )];
        // Ollama commonly declines the scripted quit; SIGTERM to the app is
        // then the only way to release the port. Matched by exact name so the
        // lowercase `ollama serve` child is not hit directly — the app has to
        // go down for the new environment to apply.
        // Best-effort like every stop command: `pkill` exits non-zero simply
        // because nothing matched (the app is already down), which is a
        // success for our purposes. The port check below is the real gate.
        plan.force_stop = vec![Cmd::optional("pkill", &["-x", "Ollama"])];
        plan.start = vec![Cmd::new("open", &["-a", "Ollama"])];
        plan.notes = vec![
            format!("Sets OLLAMA_HOST={host_port} for GUI apps (launchctl setenv), then restarts the Ollama app so it picks the value up."),
            "Ollama usually declines a scripted quit, so LocalRouter terminates the app and \
             reopens it. Any loaded model is unloaded and reloads on the next request."
                .to_string(),
            "If you start Ollama from a terminal instead, the variable won't apply there — use \
             the one-off command shown under Manual."
                .to_string(),
        ];
    } else if cfg!(target_os = "windows") {
        // `setx` writes the user-level environment variable; already-running
        // processes keep the old value, hence the explicit restart step.
        plan.configure = vec![Cmd::new("setx", &["OLLAMA_HOST", host_port])];
        plan.unconfigure = vec![Cmd::new("setx", &["OLLAMA_HOST", ""])];
        plan.manual_steps =
            vec!["Quit Ollama from the system tray and start it again.".to_string()];
        plan.notes = vec![
            format!("Sets the user environment variable OLLAMA_HOST={host_port}."),
            "Ollama must be quit from the tray and restarted — running processes don't see the \
             new value."
                .to_string(),
        ];
        plan.restart_hint = Some(
            "Quit Ollama from the tray, start it again, then click Start listener.".to_string(),
        );
    } else {
        // Linux: Ollama is typically a systemd unit, and editing it needs root.
        plan.manual_steps = vec![
            "Run `sudo systemctl edit ollama.service`.".to_string(),
            format!("Add:\n[Service]\nEnvironment=\"OLLAMA_HOST={host_port}\""),
            "Run `sudo systemctl daemon-reload && sudo systemctl restart ollama`.".to_string(),
        ];
        plan.notes = vec![
            "On Linux, Ollama usually runs as a systemd service and relocating it needs root, \
             so LocalRouter won't do it for you."
                .to_string(),
        ];
        plan.restart_hint =
            Some("After Ollama restarts on the new port, click Start listener.".to_string());
    }

    plan.notes.push(format!(
        "Apps keep pointing at port {listen_port}; Ollama itself moves to {upstream_port}."
    ));
    // Worth stating plainly: the wrap is loopback-only, so an Ollama that was
    // deliberately reachable from other machines stops being reachable.
    plan.notes.push(
        "LocalRouter listens on 127.0.0.1 only. If other machines on your network were reaching \
         this Ollama directly, they won't reach the wrapped port."
            .to_string(),
    );
    plan
}

/// LM Studio ships an `lms` CLI that can restart its server on another port.
/// Unlike Ollama it takes the port as an argument, so no environment plumbing
/// and no app restart are involved.
fn lmstudio_plan(upstream_port: u16) -> ReversePlan {
    let mut plan = ReversePlan {
        provider_label: "LM Studio".to_string(),
        oneoff_command: Some(format!("lms server start --port {upstream_port}")),
        manual_steps: vec![
            "Open LM Studio → Developer (or Local Server) tab.".to_string(),
            format!("Set the server port to {upstream_port} and restart the server."),
        ],
        restart_hint: Some(
            "LM Studio's local server must be restarted on the new port.".to_string(),
        ),
        ..Default::default()
    };

    if let Some(lms) = find_lms() {
        plan.stop = vec![Cmd::optional(&lms, &["server", "stop"])];
        plan.start = vec![Cmd::new(
            &lms,
            &["server", "start", "--port", &upstream_port.to_string()],
        )];
        // `configure` stays empty: the port is an argument to `start`, so
        // there is nothing to persist beforehand. Mark the plan as reversible
        // by giving `unconfigure` a no-op-free counterpart at call time
        // (see `lmstudio_undo`).
        plan.notes.push(format!(
            "Uses the LM Studio CLI at {lms} to restart its server on port {upstream_port}."
        ));
    } else {
        plan.notes.push(
            "The `lms` CLI wasn't found, so the server has to be moved from LM Studio's \
             interface (or install the CLI with `npx lmstudio install-cli`)."
                .to_string(),
        );
    }
    plan
}

/// Undo for LM Studio needs the original port, which `plan_for` doesn't carry
/// (it only knows where the provider is going), so it is built separately.
pub fn lmstudio_undo_plan(listen_port: u16) -> ReversePlan {
    let mut plan = ReversePlan {
        provider_label: "LM Studio".to_string(),
        ..Default::default()
    };
    if let Some(lms) = find_lms() {
        plan.stop = vec![Cmd::optional(&lms, &["server", "stop"])];
        plan.start = vec![Cmd::new(
            &lms,
            &["server", "start", "--port", &listen_port.to_string()],
        )];
    }
    plan
}

/// Locate the LM Studio CLI: on `PATH`, or at its default install location.
fn find_lms() -> Option<String> {
    if lr_utils::binary::find_binary("lms").is_some() {
        return Some("lms".to_string());
    }
    let home = dirs::home_dir()?;
    let candidate =
        home.join(".lmstudio")
            .join("bin")
            .join(if cfg!(windows) { "lms.exe" } else { "lms" });
    candidate.exists().then(|| candidate.display().to_string())
}

/// How long to wait for a port to change state during a relocation.
const PORT_WAIT: Duration = Duration::from_secs(15);
/// A restarted app needs longer to answer than a socket needs to close.
const START_WAIT: Duration = Duration::from_secs(25);
const POLL_EVERY: Duration = Duration::from_millis(250);

/// Whether anything is listening on a local TCP port right now.
async fn port_in_use(port: u16) -> bool {
    tokio::time::timeout(
        Duration::from_millis(300),
        tokio::net::TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    .is_ok_and(|r| r.is_ok())
}

/// Poll until `port` reaches `want_in_use`, or the deadline passes.
async fn wait_for_port(port: u16, want_in_use: bool, limit: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + limit;
    loop {
        if port_in_use(port).await == want_in_use {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(POLL_EVERY).await;
    }
}

/// Run one command. `ignore_failure` commands report their failure to the
/// caller rather than aborting — the caller decides, based on the *outcome*,
/// whether it mattered.
fn run_one(cmd: &Cmd) -> Result<(), String> {
    match Command::new(&cmd.program).args(&cmd.args).output() {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => {
            // Some tools report failure in stderr while still exiting 0-ish,
            // and some (osascript) exit non-zero for a refusal that is
            // survivable. Either way the message is what's useful.
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            Err(if stderr.is_empty() {
                format!("`{}` failed", cmd.display())
            } else {
                format!("`{}` failed: {stderr}", cmd.display())
            })
        }
        Err(e) => Err(format!("could not run `{}`: {e}", cmd.display())),
    }
}

/// Run a phase, stopping at the first required failure.
fn run_phase(cmds: &[Cmd]) -> Result<Vec<String>, String> {
    let mut ran = Vec::new();
    for cmd in cmds {
        match run_one(cmd) {
            Ok(()) => ran.push(cmd.display()),
            Err(e) if cmd.ignore_failure => {
                tracing::debug!("optional relocation step failed (continuing): {e}");
            }
            Err(e) => return Err(e),
        }
    }
    Ok(ran)
}

/// Move a provider between ports and **verify it actually happened**.
///
/// `free_port` is the port that must end up released (the one LocalRouter is
/// taking over, or giving back); `serve_port` is the one the provider must end
/// up answering on. Success means both were observed — never merely that the
/// commands exited zero, which is exactly how the first version of this
/// silently did nothing.
pub async fn relocate(
    plan: &ReversePlan,
    free_port: u16,
    serve_port: u16,
    configure: &[Cmd],
) -> Result<LaunchResult, String> {
    if !plan.supports_auto() {
        return Err(format!(
            "LocalRouter can't relocate {} automatically — follow the manual steps instead",
            plan.provider_label
        ));
    }

    // Already in the desired state? Re-running the wrap must be safe and
    // instant, not a needless restart of a provider that is serving fine.
    if port_in_use(serve_port).await && !port_in_use(free_port).await {
        return Ok(LaunchResult {
            success: true,
            message: format!(
                "{} is already serving on port {serve_port}; port {free_port} is free",
                plan.provider_label
            ),
            modified_files: vec![],
            backup_files: vec![],
            terminal_command: None,
        });
    }

    let mut steps = run_phase(configure)?;

    // Nothing to stop if the port is already free — the provider may simply not
    // be running, which is a perfectly normal starting point.
    if port_in_use(free_port).await && (!plan.stop.is_empty() || !plan.force_stop.is_empty()) {
        steps.extend(run_phase(&plan.stop)?);
        // The polite stop is best-effort; what counts is whether the port let go.
        if !wait_for_port(free_port, false, Duration::from_secs(5)).await
            && !plan.force_stop.is_empty()
        {
            steps.extend(run_phase(&plan.force_stop)?);
            steps.push(format!(
                "{} did not stop on request, so it was terminated",
                plan.provider_label
            ));
        }
        if !wait_for_port(free_port, false, PORT_WAIT).await {
            return Err(format!(
                "{} is still listening on port {free_port} — LocalRouter could not free it",
                plan.provider_label
            ));
        }
        steps.push(format!("Port {free_port} released"));
    }

    steps.extend(run_phase(&plan.start)?);

    if !wait_for_port(serve_port, true, START_WAIT).await {
        return Err(format!(
            "{} was restarted but is not answering on port {serve_port} yet",
            plan.provider_label
        ));
    }
    steps.push(format!(
        "{} answering on port {serve_port}",
        plan.provider_label
    ));

    Ok(LaunchResult {
        success: true,
        message: steps.join("\n"),
        modified_files: vec![],
        backup_files: vec![],
        terminal_command: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_templates_to_providers_and_ports() {
        assert_eq!(provider_key_for_template("reverse-ollama"), Some("ollama"));
        assert_eq!(
            provider_key_for_template("reverse-lmstudio"),
            Some("lmstudio")
        );
        assert_eq!(provider_key_for_template("claude-code"), None);
        assert_eq!(provider_key_for_template("reverse-unknown"), None);
        assert_eq!(default_ports("ollama"), Some((11434, 11435)));
        assert_eq!(default_ports("nope"), None);
    }

    #[test]
    fn every_default_relocates_to_a_different_port() {
        for (key, listen, upstream) in DEFAULT_PORTS {
            assert_ne!(listen, upstream, "{key} must actually move");
        }
    }

    #[test]
    fn ollama_plan_mentions_both_ports_and_offers_a_oneoff() {
        let plan = plan_for("ollama", 11434, 11435);
        assert_eq!(plan.provider_label, "Ollama");
        assert_eq!(
            plan.oneoff_command.as_deref(),
            Some("OLLAMA_HOST=127.0.0.1:11435 ollama serve")
        );
        assert!(plan.notes.iter().any(|n| n.contains("11434")));
        assert!(plan.notes.iter().any(|n| n.contains("11435")));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_ollama_relocation_is_automatic_and_reversible() {
        let plan = plan_for("ollama", 11434, 11435);
        assert!(plan.supports_auto() && plan.supports_undo());
        let cmds = plan.auto_commands();
        assert_eq!(cmds[0], "launchctl setenv OLLAMA_HOST 127.0.0.1:11435");
        assert!(
            cmds.iter().any(|c| c.contains("open -a Ollama")),
            "the app must be restarted, not just the server: launchctl setenv only \
             seeds newly launched processes"
        );
        // Undo must clear the variable, not set it to the old port — otherwise
        // an uninstall of LocalRouter would leave Ollama pinned forever.
        assert_eq!(
            plan.unconfigure.first().map(Cmd::display),
            Some("launchctl unsetenv OLLAMA_HOST".to_string())
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_ollama_has_an_escalation_because_it_refuses_scripted_quits() {
        // Ollama answers an AppleScript quit with "User canceled" (-128), so a
        // polite-stop-only plan would leave the port held and the whole wrap
        // would silently do nothing.
        let plan = plan_for("ollama", 11434, 11435);
        assert!(
            plan.stop.iter().all(|c| c.ignore_failure),
            "the scripted quit must be best-effort"
        );
        assert!(
            !plan.force_stop.is_empty(),
            "there must be an escalation when the port stays held"
        );
        // And the user must be able to see the escalation before agreeing.
        assert!(plan
            .auto_commands()
            .iter()
            .any(|c| c.contains("pkill") && c.contains("only if the port is still held")));
    }

    #[test]
    fn every_automatable_provider_is_also_reversible() {
        // Applies to all local providers, not just Ollama: if we can move it,
        // the user must be able to put it back.
        for (key, listen, upstream) in DEFAULT_PORTS {
            let plan = plan_for(key, *listen, *upstream);
            if plan.supports_auto() {
                assert!(
                    plan.supports_undo(),
                    "{key} can be relocated but not restored"
                );
                assert!(
                    !plan.start.is_empty(),
                    "{key} claims automation without a way to start the provider"
                );
            }
        }
    }

    #[test]
    fn stop_commands_are_best_effort_for_every_provider() {
        // Outcome (the port let go) is the gate — not exit codes. `pkill`
        // exits non-zero merely because nothing matched, and `lms server stop`
        // does too when no server is running; neither is a real failure.
        for (key, listen, upstream) in DEFAULT_PORTS {
            let plan = plan_for(key, *listen, *upstream);
            for cmd in plan.stop.iter().chain(plan.force_stop.iter()) {
                assert!(
                    cmd.ignore_failure,
                    "{key}: `{}` must be best-effort — the port check decides",
                    cmd.display()
                );
            }
        }
    }

    #[test]
    fn every_provider_gives_the_user_something_to_do() {
        // No provider may leave the user with a blank panel: either we automate
        // it, or we hand over a command, or we spell out the GUI steps.
        for (key, listen, upstream) in DEFAULT_PORTS {
            let plan = plan_for(key, *listen, *upstream);
            assert!(
                plan.supports_auto()
                    || plan.oneoff_command.is_some()
                    || !plan.manual_steps.is_empty(),
                "{key} offers no path forward"
            );
            assert!(!plan.provider_label.is_empty(), "{key} has no display name");
        }
    }

    #[test]
    fn gui_only_providers_offer_manual_steps_not_automation() {
        for key in ["jan", "gpt4all"] {
            let plan = plan_for(key, 1337, 1338);
            assert!(!plan.supports_auto(), "{key} must not claim automation");
            assert!(!plan.manual_steps.is_empty(), "{key} needs manual steps");
            assert!(plan.restart_hint.is_some());
        }
    }

    #[test]
    fn command_line_providers_offer_a_oneoff_command() {
        assert!(plan_for("localai", 8080, 8081)
            .oneoff_command
            .is_some_and(|c| c.contains("8081")));
        assert!(plan_for("llamacpp", 8080, 8082)
            .oneoff_command
            .is_some_and(|c| c.contains("8082")));
    }

    #[test]
    fn unknown_provider_still_produces_usable_guidance() {
        let plan = plan_for("mystery", 9000, 9001);
        assert!(!plan.supports_auto());
        assert!(plan.manual_steps[0].contains("9001"));
    }

    #[tokio::test]
    async fn relocating_a_provider_we_cannot_automate_is_an_error_not_a_silent_success() {
        // Jan has no automation; claiming success here is what would leave a
        // user with a listener bound in front of nothing.
        let plan = plan_for("jan", 1337, 1338);
        assert!(relocate(&plan, 1337, 1338, &[]).await.is_err());
    }

    #[test]
    fn automation_requires_both_a_config_change_and_a_restart() {
        // A plan that can set an env var but never restart the provider hasn't
        // relocated anything, so it must not advertise automation.
        let half = ReversePlan {
            provider_label: "Half".to_string(),
            configure: vec![Cmd::new("true", &[])],
            ..Default::default()
        };
        assert!(!half.supports_auto());

        let whole = ReversePlan {
            start: vec![Cmd::new("true", &[])],
            ..half
        };
        assert!(whole.supports_auto());
    }

    #[test]
    fn quotes_arguments_containing_spaces() {
        let cmd = Cmd::new("osascript", &["-e", "quit app \"Ollama\""]);
        assert_eq!(cmd.display(), "osascript -e 'quit app \"Ollama\"'");
    }
}
