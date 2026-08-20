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

/// The generic "wrap anything" template: the user supplies both ports, and we
/// make no assumptions about how (or whether) the provider can be restarted.
pub const CUSTOM_KEY: &str = "custom";

/// Starting ports for the generic template. Deliberately a common local-server
/// pair rather than a real provider's, since the user is expected to change
/// both — a client must be valid the moment it is created, so they can't start
/// empty.
pub const CUSTOM_DEFAULT_PORTS: (u16, u16) = (8000, 8001);

/// Client-template id → provider key (`reverse-ollama` → `ollama`,
/// `reverse-custom` → `custom`).
pub fn provider_key_for_template(template_id: &str) -> Option<&'static str> {
    let key = template_id.strip_prefix("reverse-")?;
    if key == CUSTOM_KEY {
        return Some(CUSTOM_KEY);
    }
    DEFAULT_PORTS
        .iter()
        .find(|(k, _, _)| *k == key)
        .map(|(k, _, _)| *k)
}

/// Whether this provider's ports are the user's to choose. Every preconfigured
/// provider has known ports we fill in and show read-only; only the generic
/// template asks the user.
pub fn ports_are_editable(provider_key: Option<&str>) -> bool {
    provider_key == Some(CUSTOM_KEY)
}

/// Default (listen_port, upstream_port) for a provider key.
pub fn default_ports(provider_key: &str) -> Option<(u16, u16)> {
    if provider_key == CUSTOM_KEY {
        return Some(CUSTOM_DEFAULT_PORTS);
    }
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
    /// Environment overrides for the child: `Some` sets, `None` removes.
    ///
    /// A process we spawn inherits *our* environment, not whatever `setx` or
    /// `launchctl setenv` just wrote — so a relaunched provider has to be
    /// handed the variable explicitly, and an undo relaunch has to have it
    /// taken away explicitly (LocalRouter itself may have been started after
    /// the variable was set).
    pub env: Vec<(String, Option<String>)>,
    /// Spawn without waiting for exit. Required for GUI apps: `open -a`
    /// returns at once, but launching `ollama app.exe` or a Linux binary
    /// directly would block `output()` until the user quits the app.
    pub detach: bool,
}

impl Cmd {
    fn new(program: &str, args: &[&str]) -> Self {
        Self {
            program: program.to_string(),
            args: args.iter().map(|a| a.to_string()).collect(),
            ignore_failure: false,
            env: Vec::new(),
            detach: false,
        }
    }

    fn optional(program: &str, args: &[&str]) -> Self {
        Self {
            ignore_failure: true,
            ..Self::new(program, args)
        }
    }

    /// A GUI app launch: detached, with the given environment overrides.
    fn launch(program: &str, args: &[&str], env: &[(&str, Option<&str>)]) -> Self {
        Self {
            detach: true,
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.map(str::to_string)))
                .collect(),
            ..Self::new(program, args)
        }
    }

    /// Human-readable form, for the UI and for error messages.
    pub fn display(&self) -> String {
        let env = self
            .env
            .iter()
            .map(|(k, v)| match v {
                Some(v) => format!("{k}={v} "),
                None => format!("{k}= "),
            })
            .collect::<String>();
        let program = if self.program.contains(' ') {
            format!("'{}'", self.program)
        } else {
            self.program.clone()
        };
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
            format!("{env}{program}")
        } else {
            format!("{env}{program} {args}")
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
    /// `configure` (and `unconfigure`) already restart the provider, so there
    /// is no separate `start`. Used where every step needs elevation
    /// (systemd on Linux): `pkexec` does not cache credentials, so three
    /// phases would mean three password prompts — one transaction is one.
    pub configure_restarts: bool,
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
    /// Start for the *undo* direction, when it must differ from `start` —
    /// a relaunch that passes the variable explicitly has to pass its
    /// absence just as explicitly on the way back. Empty = reuse `start`.
    pub undo_start: Vec<Cmd>,
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
        if self.configure_restarts {
            return !self.configure.is_empty();
        }
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

    /// The same plan pointed the other way: `unconfigure` becomes the
    /// configuration step and `undo_start` (if any) the relaunch. Feeding this
    /// to [`relocate`] with the ports swapped moves the provider home.
    pub fn into_undo(self) -> ReversePlan {
        let start = if self.undo_start.is_empty() {
            self.start.clone()
        } else {
            self.undo_start.clone()
        };
        ReversePlan {
            configure: self.unconfigure.clone(),
            unconfigure: self.configure.clone(),
            start,
            undo_start: Vec::new(),
            ..self
        }
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
        CUSTOM_KEY => ReversePlan {
            provider_label: "your provider".to_string(),
            manual_steps: vec![
                format!("Restart the server that currently owns port {listen_port} so it listens on {upstream_port} instead."),
                format!("Leave everything else as-is — your apps keep using port {listen_port}."),
                "Then start the listener below.".to_string(),
            ],
            notes: vec![
                "LocalRouter doesn't know how to restart this server, so move it yourself and \
                 then bind the port here."
                    .to_string(),
                format!("Anything speaking HTTP on port {listen_port} can be wrapped — the \
                         request is forwarded byte for byte."),
            ],
            restart_hint: Some(format!(
                "Once something is answering on port {upstream_port}, click Start listener."
            )),
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
        oneoff_command: Some(ollama_oneoff(host_port)),
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
        windows_ollama_plan(&mut plan, host_port);
    } else {
        linux_ollama_plan(&mut plan, host_port);
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

/// `ollama serve` on the new port, for the shell the user actually has.
fn ollama_oneoff(host_port: &str) -> String {
    if cfg!(target_os = "windows") {
        // PowerShell is the default shell on Windows 10/11; `VAR=value cmd`
        // is a Unix-ism that errors there.
        format!("$env:OLLAMA_HOST=\"{host_port}\"; ollama serve")
    } else {
        format!("OLLAMA_HOST={host_port} ollama serve")
    }
}

/// Windows: the Ollama installer puts the tray app at
/// `%LOCALAPPDATA%\\Programs\\Ollama\\ollama app.exe` and registers it to run
/// at login. `setx` persists `OLLAMA_HOST` for future launches (Explorer picks
/// up the change via the broadcast `setx` sends), and the relaunch we do
/// ourselves passes the variable explicitly because a child inherits *our*
/// environment, not the registry.
fn windows_ollama_plan(plan: &mut ReversePlan, host_port: &str) {
    plan.manual_steps = vec![
        format!("Set the user environment variable OLLAMA_HOST to {host_port} (Settings → System → About → Advanced system settings → Environment Variables)."),
        "Quit Ollama from the system tray and start it again.".to_string(),
    ];
    plan.restart_hint =
        Some("Quit Ollama from the tray, start it again, then click Start listener.".to_string());

    let Some(app) = find_ollama_app_windows() else {
        plan.notes.push(
            "The Ollama app wasn't found under %LOCALAPPDATA%\\Programs\\Ollama, so LocalRouter \
             can't restart it for you — set the variable and restart Ollama yourself."
                .to_string(),
        );
        return;
    };

    plan.configure = vec![Cmd::new("setx", &["OLLAMA_HOST", host_port])];
    // `setx VAR ""` leaves an empty variable behind; deleting the registry
    // value is the real inverse. Best-effort: the value may already be gone.
    plan.unconfigure = vec![Cmd::optional(
        "reg",
        &["delete", "HKCU\\Environment", "/v", "OLLAMA_HOST", "/f"],
    )];
    // Without /F `taskkill` asks the app to close; a tray app without a
    // window usually ignores that, so the forced kill is the one that counts.
    // Both the tray app and the server child are named, so a stray
    // `ollama.exe` holding the port without its parent is caught too.
    plan.stop = vec![Cmd::optional("taskkill", &["/IM", "ollama app.exe"])];
    plan.force_stop = vec![
        Cmd::optional("taskkill", &["/F", "/IM", "ollama app.exe"]),
        Cmd::optional("taskkill", &["/F", "/IM", "ollama.exe"]),
    ];
    plan.start = vec![Cmd::launch(&app, &[], &[("OLLAMA_HOST", Some(host_port))])];
    // Moving back must relaunch *without* the variable — and LocalRouter's
    // own environment may still carry it, so it is removed, not just omitted.
    plan.undo_start = vec![Cmd::launch(&app, &[], &[("OLLAMA_HOST", None)])];
    plan.notes = vec![
        format!("Sets the user environment variable OLLAMA_HOST={host_port} (setx), then restarts the Ollama tray app with it."),
        "Ollama's tray app ignores a polite close, so LocalRouter terminates it and starts it \
         again. Any loaded model is unloaded and reloads on the next request."
            .to_string(),
        "If you run `ollama serve` from a terminal instead, open a new terminal after this so \
         it sees the variable — or use the one-off command shown under Manual."
            .to_string(),
    ];
}

/// `%LOCALAPPDATA%\\Programs\\Ollama\\ollama app.exe`, if installed.
fn find_ollama_app_windows() -> Option<String> {
    if !cfg!(target_os = "windows") {
        return None;
    }
    let base = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from)?;
    let candidate = base.join("Programs").join("Ollama").join("ollama app.exe");
    candidate.exists().then(|| candidate.display().to_string())
}

/// Where the systemd drop-in for Ollama goes. A drop-in, not an edit of the
/// unit: it survives Ollama's own upgrades (which rewrite the unit) and undo
/// is a file removal rather than a diff.
const LINUX_OLLAMA_DROPIN: &str = "/etc/systemd/system/ollama.service.d/localrouter.conf";

/// Linux: Ollama's installer sets it up as a system-wide systemd unit, and
/// changing a system unit needs root. `pkexec` is the desktop way to ask for
/// it (one password dialog via the session's polkit agent), so when both the
/// unit and `pkexec` are present the whole move is one elevated transaction:
/// write the drop-in, reload, restart. Otherwise — a hand-run `ollama serve`,
/// a headless box without polkit — the user gets the exact commands instead.
fn linux_ollama_plan(plan: &mut ReversePlan, host_port: &str) {
    plan.manual_steps = vec![
        "Run `sudo systemctl edit ollama.service`.".to_string(),
        format!("Add:\n[Service]\nEnvironment=\"OLLAMA_HOST={host_port}\""),
        "Run `sudo systemctl daemon-reload && sudo systemctl restart ollama`.".to_string(),
    ];
    plan.restart_hint =
        Some("After Ollama restarts on the new port, click Start listener.".to_string());

    if !linux_ollama_unit_exists() {
        plan.notes.push(
            "No ollama.service systemd unit was found, so Ollama is probably started by hand \
             — restart it with the one-off command shown under Manual."
                .to_string(),
        );
        return;
    }
    if lr_utils::binary::find_binary("pkexec").is_none() {
        plan.notes.push(
            "Relocating the ollama.service unit needs root and `pkexec` isn't available for a \
             password prompt, so run the steps under Manual yourself."
                .to_string(),
        );
        return;
    }

    // host_port is built by us from a u16, so it is safe to inline. The
    // script is `sh -c` so one pkexec prompt covers write + reload + restart.
    let dir = LINUX_OLLAMA_DROPIN
        .rsplit_once('/')
        .map(|(d, _)| d)
        .unwrap_or("/");
    let apply = format!(
        "mkdir -p {dir} && printf '[Service]\\nEnvironment=OLLAMA_HOST={host_port}\\n' > {LINUX_OLLAMA_DROPIN} && systemctl daemon-reload && systemctl restart ollama"
    );
    let revert = format!(
        "rm -f {LINUX_OLLAMA_DROPIN} && systemctl daemon-reload && systemctl restart ollama"
    );
    plan.configure = vec![Cmd::new("pkexec", &["sh", "-c", &apply])];
    plan.unconfigure = vec![Cmd::new("pkexec", &["sh", "-c", &revert])];
    plan.configure_restarts = true;
    plan.notes = vec![
        format!("Asks for your password once (pkexec), writes {LINUX_OLLAMA_DROPIN} with OLLAMA_HOST={host_port}, and restarts the ollama systemd service."),
        "Undo removes that file and restarts the service again — nothing else in the unit is \
         touched."
            .to_string(),
        "Any loaded model is unloaded by the restart and reloads on the next request."
            .to_string(),
    ];
}

/// Whether an `ollama.service` unit is installed (system-wide, where the
/// official installer puts it).
fn linux_ollama_unit_exists() -> bool {
    if !cfg!(target_os = "linux") {
        return false;
    }
    [
        "/etc/systemd/system/ollama.service",
        "/usr/lib/systemd/system/ollama.service",
        "/lib/systemd/system/ollama.service",
    ]
    .iter()
    .any(|p| std::path::Path::new(p).exists())
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
    let exe = if cfg!(windows) { "lms.exe" } else { "lms" };
    // `~/.lmstudio/bin` is the current install location on every OS;
    // `~/.cache/lm-studio/bin` is where older releases put it.
    [
        home.join(".lmstudio").join("bin").join(exe),
        home.join(".cache").join("lm-studio").join("bin").join(exe),
    ]
    .into_iter()
    .find(|c| c.exists())
    .map(|c| c.display().to_string())
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
    let mut command = Command::new(&cmd.program);
    command.args(&cmd.args);
    for (k, v) in &cmd.env {
        match v {
            Some(v) => command.env(k, v),
            None => command.env_remove(k),
        };
    }
    if cmd.detach {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP: the app must
            // outlive LocalRouter and not share its (non-existent) console.
            command.creation_flags(0x0000_0008 | 0x0000_0200);
        }
        return command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(drop)
            .map_err(|e| format!("could not start `{}`: {e}", cmd.display()));
    }
    match command.output() {
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

    if plan.configure_restarts {
        // The restart happened inside `configure`; the service may still be
        // letting go of the old socket. Binding it while it is held would be
        // blamed on the provider by the listener, so wait here instead.
        if !wait_for_port(free_port, false, PORT_WAIT).await {
            return Err(format!(
                "{} was restarted but is still listening on port {free_port}",
                plan.provider_label
            ));
        }
        steps.push(format!("Port {free_port} released"));
    }

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

    if !plan.configure_restarts {
        steps.extend(run_phase(&plan.start)?);
    }

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
    fn generic_template_lets_the_user_pick_the_ports() {
        assert_eq!(
            provider_key_for_template("reverse-custom"),
            Some(CUSTOM_KEY)
        );
        assert_eq!(default_ports(CUSTOM_KEY), Some(CUSTOM_DEFAULT_PORTS));
        // A new client must be valid immediately, so the defaults can't be zero
        // and can't collide with each other.
        let (listen, upstream) = CUSTOM_DEFAULT_PORTS;
        assert!(listen > 0 && upstream > 0);
        assert_ne!(listen, upstream);

        // Only the generic template exposes the ports for editing.
        assert!(ports_are_editable(Some(CUSTOM_KEY)));
        for (key, _, _) in DEFAULT_PORTS {
            assert!(
                !ports_are_editable(Some(key)),
                "{key} has known ports and must show them read-only"
            );
        }
        assert!(!ports_are_editable(None));
    }

    #[test]
    fn generic_plan_explains_the_move_without_claiming_automation() {
        let plan = plan_for(CUSTOM_KEY, 8000, 8001);
        assert!(
            !plan.supports_auto(),
            "we know nothing about how to restart an arbitrary server"
        );
        assert!(!plan.manual_steps.is_empty());
        // The steps must name both ports, or they're not actionable.
        let joined = plan.manual_steps.join(" ");
        assert!(
            joined.contains("8000") && joined.contains("8001"),
            "{joined}"
        );
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
        let expected = if cfg!(target_os = "windows") {
            "$env:OLLAMA_HOST=\"127.0.0.1:11435\"; ollama serve"
        } else {
            "OLLAMA_HOST=127.0.0.1:11435 ollama serve"
        };
        assert_eq!(plan.oneoff_command.as_deref(), Some(expected));
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
                    !plan.start.is_empty() || plan.configure_restarts,
                    "{key} claims automation without a way to start the provider"
                );
                if plan.configure_restarts {
                    assert!(
                        !plan.unconfigure.is_empty(),
                        "{key}: a configure-restarts plan must undo by the same route"
                    );
                }
            }
        }
    }

    #[test]
    fn every_provider_has_a_plan_on_this_os() {
        // The point of the exercise: no provider may be a macOS special case.
        // On every OS each one either automates or explains itself, and the
        // explanation must name the port the provider is moving to.
        for (key, listen, upstream) in DEFAULT_PORTS {
            let plan = plan_for(key, *listen, *upstream);
            let text = plan
                .manual_steps
                .iter()
                .chain(plan.notes.iter())
                .chain(plan.oneoff_command.iter())
                .cloned()
                .collect::<Vec<_>>()
                .join(" ");
            assert!(
                plan.supports_auto() || text.contains(&upstream.to_string()),
                "{key}: manual guidance on this OS never mentions port {upstream}: {text}"
            );
        }
    }

    #[test]
    fn gui_launches_are_detached_and_carry_their_environment() {
        // A relaunch must (a) not block until the user quits the app, and
        // (b) hand the provider the variable explicitly — the child inherits
        // LocalRouter's environment, not what setx/launchctl just wrote.
        for (key, listen, upstream) in DEFAULT_PORTS {
            let plan = plan_for(key, *listen, *upstream);
            for cmd in plan.start.iter().filter(|c| c.detach) {
                assert!(
                    !cmd.env.is_empty() || cfg!(target_os = "macos"),
                    "{key}: detached launch `{}` relies on inherited environment",
                    cmd.display()
                );
            }
        }
    }

    #[test]
    fn undo_relaunches_without_the_variable_it_set() {
        // The Windows bug this guards: reusing `start` for undo would hand
        // the relaunched app OLLAMA_HOST=<new port> and move it nowhere.
        let plan = ReversePlan {
            provider_label: "App".to_string(),
            configure: vec![Cmd::new("setx", &["V", "1"])],
            unconfigure: vec![Cmd::new("reg", &["delete", "V"])],
            start: vec![Cmd::launch("app", &[], &[("V", Some("1"))])],
            undo_start: vec![Cmd::launch("app", &[], &[("V", None)])],
            ..Default::default()
        };
        let undo = plan.clone().into_undo();
        assert_eq!(undo.configure, plan.unconfigure);
        assert_eq!(undo.start[0].env, vec![("V".to_string(), None)]);
        assert!(undo.supports_auto());

        // No undo_start: the same relaunch serves both directions (macOS).
        let same = ReversePlan {
            undo_start: vec![],
            ..plan
        }
        .into_undo();
        assert_eq!(
            same.start[0].env,
            vec![("V".to_string(), Some("1".to_string()))]
        );
    }

    #[test]
    fn configure_restarts_plans_need_no_separate_start() {
        let plan = ReversePlan {
            provider_label: "Unit".to_string(),
            configure: vec![Cmd::new("true", &[])],
            unconfigure: vec![Cmd::new("true", &[])],
            configure_restarts: true,
            ..Default::default()
        };
        assert!(plan.supports_auto() && plan.supports_undo());
        // …but an empty configure is still nothing.
        let empty = ReversePlan {
            configure: vec![],
            ..plan
        };
        assert!(!empty.supports_auto());
    }

    #[test]
    fn display_shows_environment_and_quotes_spaced_programs() {
        let cmd = Cmd::launch(
            r"C:\Users\me\AppData\Local\Programs\Ollama\ollama app.exe",
            &[],
            &[("OLLAMA_HOST", Some("127.0.0.1:11435"))],
        );
        assert_eq!(
            cmd.display(),
            r"OLLAMA_HOST=127.0.0.1:11435 'C:\Users\me\AppData\Local\Programs\Ollama\ollama app.exe'"
        );
        let unset = Cmd::launch("app", &[], &[("OLLAMA_HOST", None)]);
        assert_eq!(unset.display(), "OLLAMA_HOST= app");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_ollama_uses_setx_and_relaunches_with_the_variable() {
        let plan = plan_for("ollama", 11434, 11435);
        assert!(plan
            .oneoff_command
            .as_deref()
            .is_some_and(|c| c.starts_with("$env:OLLAMA_HOST=")));
        if !plan.supports_auto() {
            // No Ollama installed on this machine: must fall back to manual,
            // not to a half plan.
            assert!(plan.configure.is_empty() && plan.start.is_empty());
            assert!(!plan.manual_steps.is_empty());
            return;
        }
        let cmds = plan.auto_commands();
        assert_eq!(cmds[0], "setx OLLAMA_HOST 127.0.0.1:11435");
        assert!(cmds
            .iter()
            .any(|c| c.contains("taskkill /F /IM 'ollama app.exe'")
                && c.contains("only if the port is still held")));
        let start = &plan.start[0];
        assert!(start.detach && start.program.ends_with("ollama app.exe"));
        assert_eq!(
            start.env,
            vec![(
                "OLLAMA_HOST".to_string(),
                Some("127.0.0.1:11435".to_string())
            )]
        );
        // Undo deletes the value (setx "" would leave an empty variable).
        assert!(plan.unconfigure[0].display().starts_with("reg delete"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_ollama_is_one_pkexec_transaction_or_manual() {
        let plan = plan_for("ollama", 11434, 11435);
        assert!(plan
            .manual_steps
            .iter()
            .any(|s| s.contains("systemctl edit")));
        if !plan.supports_auto() {
            assert!(plan.configure.is_empty());
            return;
        }
        assert!(plan.configure_restarts);
        assert!(
            plan.start.is_empty(),
            "start would be a second password prompt"
        );
        assert_eq!(plan.configure.len(), 1);
        assert_eq!(plan.configure[0].program, "pkexec");
        let script = plan.configure[0].args.last().unwrap();
        assert!(script.contains(LINUX_OLLAMA_DROPIN));
        assert!(script.contains("OLLAMA_HOST=127.0.0.1:11435"));
        assert!(script.contains("systemctl restart ollama"));
        let undo = plan.unconfigure[0].args.last().unwrap();
        assert!(undo.contains(&format!("rm -f {LINUX_OLLAMA_DROPIN}")));
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
