//! Detecting the Flatpak / Snap sandboxes and escaping them to reach the host.
//!
//! LocalRouter is only useful if it can run the user's own tooling: MCP stdio
//! servers (`npx`, `uvx`, …) in [`lr-mcp`] and coding agents (`claude`,
//! `aider`, …) in [`lr-coding-agents`]. Those binaries live on the *host*, not
//! inside our package.
//!
//! - **Flatpak** confines us to the runtime's filesystem, so `npx` simply does
//!   not exist inside the sandbox. Reaching it requires proxying through
//!   `flatpak-spawn --host`, which needs `--talk-name=org.freedesktop.Flatpak`
//!   in the manifest.
//! - **Snap** is packaged with `confinement: classic` (the same precedent as
//!   VS Code, another dev tool that must drive host toolchains). Classic snaps
//!   run unconfined, so no rewriting is needed — detection exists only so the
//!   updater can tell that the Snap Store owns this install.
//!
//! Everything here is a pure function over injected inputs, with thin cached
//! wrappers that read the real process environment. That keeps the precedence
//! rules testable from macOS, where neither sandbox exists.

use std::path::Path;
use std::sync::OnceLock;

/// Which application sandbox, if any, this process is confined by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sandbox {
    /// Running directly on the host.
    None,
    /// Running inside a Flatpak sandbox.
    Flatpak,
    /// Running inside a Snap (classic confinement — unconfined in practice).
    Snap,
}

impl Sandbox {
    /// Whether spawning a host binary requires proxying through a helper.
    ///
    /// Only Flatpak does. Classic-confinement snaps already see the host
    /// filesystem and `PATH`.
    pub fn needs_host_proxy(self) -> bool {
        matches!(self, Sandbox::Flatpak)
    }
}

static SANDBOX: OnceLock<Sandbox> = OnceLock::new();

/// The sandbox confining this process, detected once and cached.
pub fn current() -> Sandbox {
    *SANDBOX.get_or_init(|| {
        detect_sandbox(
            |key| std::env::var(key).ok(),
            // `/.flatpak-info` is created by flatpak inside every sandbox and
            // is the check the flatpak docs themselves recommend: it survives
            // `env -i`, which FLATPAK_ID does not.
            Path::new("/.flatpak-info").exists(),
        )
    })
}

/// Whether this process is confined by Flatpak.
pub fn is_flatpak() -> bool {
    current() == Sandbox::Flatpak
}

/// Whether this process was installed as a Snap.
pub fn is_snap() -> bool {
    current() == Sandbox::Snap
}

/// Pure sandbox detection, parameterised over its inputs for testing.
///
/// Flatpak wins over Snap: the two are never both true in practice, and if a
/// broken environment claims both, the Flatpak answer is the conservative one
/// because it triggers host-proxying rather than assuming direct access.
pub(crate) fn detect_sandbox<F>(env: F, flatpak_info_exists: bool) -> Sandbox
where
    F: Fn(&str) -> Option<String>,
{
    if flatpak_info_exists || env("FLATPAK_ID").is_some_and(|v| !v.is_empty()) {
        return Sandbox::Flatpak;
    }

    // Snap sets SNAP to the revision's mount point. `SNAP_NAME` alone is not
    // enough — it also leaks into shells spawned *by* a snap.
    if env("SNAP").is_some_and(|v| !v.is_empty()) {
        return Sandbox::Snap;
    }

    Sandbox::None
}

/// A command rewritten so that it executes on the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostInvocation {
    /// The program to actually exec.
    pub program: String,
    /// Arguments to place *before* the caller's own arguments.
    pub leading_args: Vec<String>,
    /// Environment variables to set on the spawned process.
    ///
    /// When sandboxed this is empty: the variables have already been folded
    /// into `leading_args` as `--env=` flags, because setting them on the
    /// `flatpak-spawn` process itself would configure the proxy rather than
    /// the host program.
    pub envs: Vec<(String, String)>,
}

/// Rewrite a command so it runs on the host, escaping the Flatpak sandbox.
///
/// Callers **must** route environment variables and the working directory
/// through this function instead of `Command::env`/`Command::current_dir`.
/// `flatpak-spawn` does not forward the sandbox's environment or cwd to the
/// host process, so anything set on the proxy process is silently lost.
///
/// Outside a sandbox this is a pass-through: the program and environment come
/// back unchanged and `leading_args` is empty.
pub fn host_invocation<I>(program: &str, envs: I, cwd: Option<&Path>) -> HostInvocation
where
    I: IntoIterator<Item = (String, String)>,
{
    build_host_invocation(current(), program, envs, cwd)
}

/// Pure form of [`host_invocation`], parameterised over the sandbox.
pub(crate) fn build_host_invocation<I>(
    sandbox: Sandbox,
    program: &str,
    envs: I,
    cwd: Option<&Path>,
) -> HostInvocation
where
    I: IntoIterator<Item = (String, String)>,
{
    let envs: Vec<(String, String)> = envs.into_iter().collect();

    if !sandbox.needs_host_proxy() {
        return HostInvocation {
            program: program.to_string(),
            leading_args: Vec::new(),
            envs,
        };
    }

    // `--watch-bus` makes the host process exit when our D-Bus connection
    // drops. Without it, `kill_on_drop` only reaps the local `flatpak-spawn`
    // proxy and the real MCP server or agent is orphaned on the host.
    let mut leading_args = vec!["--host".to_string(), "--watch-bus".to_string()];

    // `--directory` landed in flatpak 1.11 (2021). Without it the host process
    // inherits the portal's cwd (the user's home), which breaks any agent that
    // resolves paths relative to the project directory.
    if let Some(cwd) = cwd {
        leading_args.push(format!("--directory={}", cwd.display()));
    }

    for (key, value) in envs {
        leading_args.push(format!("--env={key}={value}"));
    }

    // The program name is just another argument to flatpak-spawn, and must
    // come last so the caller's own args follow it.
    leading_args.push(program.to_string());

    HostInvocation {
        program: "flatpak-spawn".to_string(),
        leading_args,
        envs: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_from<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        let map: HashMap<&str, &str> = pairs.iter().copied().collect();
        move |key| map.get(key).map(|v| v.to_string())
    }

    #[test]
    fn no_sandbox_when_nothing_is_set() {
        assert_eq!(detect_sandbox(env_from(&[]), false), Sandbox::None);
    }

    #[test]
    fn flatpak_info_file_alone_is_enough() {
        assert_eq!(detect_sandbox(env_from(&[]), true), Sandbox::Flatpak);
    }

    #[test]
    fn flatpak_id_env_is_enough() {
        assert_eq!(
            detect_sandbox(env_from(&[("FLATPAK_ID", "ai.localrouter.app")]), false),
            Sandbox::Flatpak
        );
    }

    #[test]
    fn empty_flatpak_id_is_not_a_sandbox() {
        assert_eq!(
            detect_sandbox(env_from(&[("FLATPAK_ID", "")]), false),
            Sandbox::None
        );
    }

    #[test]
    fn snap_env_detected() {
        assert_eq!(
            detect_sandbox(env_from(&[("SNAP", "/snap/localrouter/42")]), false),
            Sandbox::Snap
        );
    }

    #[test]
    fn snap_name_alone_does_not_count() {
        // SNAP_NAME leaks into shells spawned by a snap; only SNAP means we
        // *are* the snap.
        assert_eq!(
            detect_sandbox(env_from(&[("SNAP_NAME", "localrouter")]), false),
            Sandbox::None
        );
    }

    #[test]
    fn flatpak_wins_over_snap() {
        assert_eq!(
            detect_sandbox(
                env_from(&[("FLATPAK_ID", "ai.localrouter.app"), ("SNAP", "/snap/x/1")]),
                false
            ),
            Sandbox::Flatpak
        );
    }

    #[test]
    fn only_flatpak_needs_a_host_proxy() {
        assert!(Sandbox::Flatpak.needs_host_proxy());
        assert!(!Sandbox::Snap.needs_host_proxy());
        assert!(!Sandbox::None.needs_host_proxy());
    }

    #[test]
    fn unsandboxed_invocation_is_a_passthrough() {
        let inv = build_host_invocation(
            Sandbox::None,
            "npx",
            [("FOO".to_string(), "bar".to_string())],
            Some(Path::new("/work")),
        );
        assert_eq!(inv.program, "npx");
        assert!(inv.leading_args.is_empty());
        assert_eq!(inv.envs, vec![("FOO".to_string(), "bar".to_string())]);
    }

    #[test]
    fn classic_snap_is_also_a_passthrough() {
        let inv = build_host_invocation(Sandbox::Snap, "claude", [], None);
        assert_eq!(inv.program, "claude");
        assert!(inv.leading_args.is_empty());
    }

    #[test]
    fn flatpak_invocation_proxies_through_flatpak_spawn() {
        let inv = build_host_invocation(
            Sandbox::Flatpak,
            "npx",
            [("FOO".to_string(), "bar".to_string())],
            Some(Path::new("/work/project")),
        );
        assert_eq!(inv.program, "flatpak-spawn");
        assert_eq!(
            inv.leading_args,
            vec![
                "--host".to_string(),
                "--watch-bus".to_string(),
                "--directory=/work/project".to_string(),
                "--env=FOO=bar".to_string(),
                "npx".to_string(),
            ]
        );
        // Envs moved into --env flags; setting them on the proxy would be lost.
        assert!(inv.envs.is_empty());
    }

    #[test]
    fn flatpak_invocation_without_cwd_omits_directory() {
        let inv = build_host_invocation(Sandbox::Flatpak, "claude", [], None);
        assert_eq!(
            inv.leading_args,
            vec![
                "--host".to_string(),
                "--watch-bus".to_string(),
                "claude".to_string()
            ]
        );
    }

    #[test]
    fn program_is_the_last_leading_arg_so_caller_args_follow() {
        let inv = build_host_invocation(
            Sandbox::Flatpak,
            "uvx",
            [
                ("A".to_string(), "1".to_string()),
                ("B".to_string(), "2".to_string()),
            ],
            Some(Path::new("/tmp")),
        );
        assert_eq!(inv.leading_args.last().unwrap(), "uvx");
    }
}
