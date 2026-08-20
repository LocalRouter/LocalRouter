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
#[derive(Debug, Clone, Default)]
pub struct ReversePlan {
    /// Display name of the wrapped provider ("Ollama").
    pub provider_label: String,
    /// Commands LocalRouter can run to relocate the provider.
    pub auto: Vec<Cmd>,
    /// Commands that put the provider back on its original port.
    pub undo: Vec<Cmd>,
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
    pub fn supports_auto(&self) -> bool {
        !self.auto.is_empty()
    }
    pub fn supports_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    /// The auto commands as display strings, so the UI can show exactly what
    /// LocalRouter would run before the user agrees to it.
    pub fn auto_commands(&self) -> Vec<String> {
        self.auto.iter().map(Cmd::display).collect()
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
        plan.auto = vec![
            Cmd::new("launchctl", &["setenv", "OLLAMA_HOST", host_port]),
            Cmd::optional("osascript", &["-e", "quit app \"Ollama\""]),
            Cmd::new("open", &["-a", "Ollama"]),
        ];
        plan.undo = vec![
            Cmd::new("launchctl", &["unsetenv", "OLLAMA_HOST"]),
            Cmd::optional("osascript", &["-e", "quit app \"Ollama\""]),
            Cmd::new("open", &["-a", "Ollama"]),
        ];
        plan.notes = vec![
            format!("Sets OLLAMA_HOST={host_port} for GUI apps (launchctl setenv) and restarts the Ollama app."),
            "If you start Ollama from a terminal instead, the variable won't apply there — use \
             the one-off command shown under Manual."
                .to_string(),
        ];
    } else if cfg!(target_os = "windows") {
        // `setx` writes the user-level environment variable; already-running
        // processes keep the old value, hence the explicit restart step.
        plan.auto = vec![Cmd::new("setx", &["OLLAMA_HOST", host_port])];
        plan.undo = vec![Cmd::new("setx", &["OLLAMA_HOST", ""])];
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
        "LocalRouter listens on 127.0.0.1 only. If other machines on your network were reaching          this Ollama directly, they won't reach the wrapped port."
            .to_string(),
    );
    plan
}

/// LM Studio ships an `lms` CLI that can restart its server on another port.
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
        plan.auto = vec![
            Cmd::optional(&lms, &["server", "stop"]),
            Cmd::new(
                &lms,
                &["server", "start", "--port", &upstream_port.to_string()],
            ),
        ];
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
/// (it only knows where the provider is going). Build it explicitly.
pub fn lmstudio_undo(listen_port: u16) -> Vec<Cmd> {
    match find_lms() {
        Some(lms) => vec![
            Cmd::optional(&lms, &["server", "stop"]),
            Cmd::new(
                &lms,
                &["server", "start", "--port", &listen_port.to_string()],
            ),
        ],
        None => vec![],
    }
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

/// Run a relocation sequence. Stops at the first required command that fails,
/// reporting which one — a half-applied relocation is worse than a clear error.
pub fn run(commands: &[Cmd], label: &str) -> Result<LaunchResult, String> {
    if commands.is_empty() {
        return Err(format!(
            "LocalRouter can't relocate {label} automatically — follow the manual steps instead"
        ));
    }

    let mut ran = Vec::new();
    for cmd in commands {
        let output = Command::new(&cmd.program).args(&cmd.args).output();
        match output {
            Ok(out) if out.status.success() => ran.push(cmd.display()),
            Ok(out) => {
                if cmd.ignore_failure {
                    tracing::debug!("optional step failed (ignored): {}", cmd.display());
                    continue;
                }
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                return Err(format!(
                    "`{}` failed{}",
                    cmd.display(),
                    if stderr.is_empty() {
                        String::new()
                    } else {
                        format!(": {stderr}")
                    }
                ));
            }
            Err(e) => {
                if cmd.ignore_failure {
                    continue;
                }
                return Err(format!("could not run `{}`: {e}", cmd.display()));
            }
        }
    }

    Ok(LaunchResult {
        success: true,
        message: format!("{label}: {}", ran.join(", ")),
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
        assert!(cmds.iter().any(|c| c.contains("open -a Ollama")));
        // Undo must clear the variable, not set it to the old port — otherwise
        // an uninstall of LocalRouter would leave Ollama pinned forever.
        assert_eq!(
            plan.undo.first().map(Cmd::display),
            Some("launchctl unsetenv OLLAMA_HOST".to_string())
        );
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

    #[test]
    fn running_an_empty_sequence_is_an_error_not_a_silent_success() {
        assert!(run(&[], "Jan").is_err());
    }

    #[test]
    fn quotes_arguments_containing_spaces() {
        let cmd = Cmd::new("osascript", &["-e", "quit app \"Ollama\""]);
        assert_eq!(cmd.display(), "osascript -e 'quit app \"Ollama\"'");
    }
}
