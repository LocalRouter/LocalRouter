//! Locating executables the way the user's terminal does.
//!
//! GUI processes do not inherit the PATH the user sees in a terminal:
//!
//! - a macOS `.app` launched from Finder/Dock starts with roughly
//!   `/usr/bin:/bin:/usr/sbin:/sbin`;
//! - a Linux `.desktop` launch inherits the session manager's PATH, which is
//!   set before the login shell sources `~/.zshrc` / `~/.profile`.
//!
//! Either way the directories where modern dev tooling actually installs —
//! `~/.local/bin`, `~/.opencode/bin`, fnm/nvm/volta node dirs, `~/.cargo/bin`,
//! `~/.bun/bin`, `/opt/homebrew/bin` — are missing, so a bare
//! [`which::which`] reports tools as "not installed" that run fine in the
//! user's terminal.
//!
//! [`find_binary`] resolves against the login-shell PATH (cached) and then a
//! small set of well-known user-local install directories.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// How long to wait for the login shell to print its PATH before giving up.
///
/// A misconfigured profile can block forever (e.g. one that prompts for
/// input); detection must not hang the app because of it.
#[cfg(unix)]
const SHELL_PATH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

static SHELL_ENV: RwLock<Option<HashMap<String, String>>> = RwLock::new(None);

/// Cached environment (currently just `PATH`) for locating and spawning
/// user-installed tools.
pub fn shell_env() -> HashMap<String, String> {
    if let Some(env) = SHELL_ENV.read().ok().and_then(|g| g.clone()) {
        return env;
    }
    refresh_shell_env()
}

/// Rebuild the cached environment, e.g. after the user installed a tool
/// while the app was running (a "Refresh" button). On Windows this re-reads
/// PATH from the registry, which is where installers such as WinGet write it;
/// the running process never sees those changes otherwise.
pub fn refresh_shell_env() -> HashMap<String, String> {
    let env = build_shell_env();
    if let Ok(mut guard) = SHELL_ENV.write() {
        *guard = Some(env.clone());
    }
    env
}

/// The user's login-shell `PATH`, falling back to the process `PATH`.
pub fn shell_path() -> Option<String> {
    shell_env().get("PATH").cloned()
}

fn build_shell_env() -> HashMap<String, String> {
    let mut env = HashMap::new();

    let path = login_shell_path()
        .or_else(windows_registry_path)
        .or_else(|| std::env::var("PATH").ok());

    if let Some(path) = path {
        env.insert("PATH".to_string(), path);
    }

    env
}

/// Ask the user's login shell for its `PATH`.
///
/// Runs `$SHELL -lic 'echo $PATH'`: `-l` sources the login profile, `-i` the
/// interactive rc file. Both are needed because tool installers write their
/// PATH export to either one depending on the shell.
///
/// Returns `None` on Windows, where GUI processes already inherit the user's
/// full environment from the registry.
#[cfg(unix)]
fn login_shell_path() -> Option<String> {
    use std::process::{Command, Stdio};

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());

    // Inside a Flatpak sandbox the login shell we can reach is the runtime's,
    // whose PATH describes the runtime rather than the user's machine. Ask the
    // host's shell instead — that is the PATH every tool we spawn will see.
    let invocation = crate::sandbox::host_invocation(&shell, [], None);

    // Discard stdin so an interactive profile that tries to read from the
    // terminal gets EOF instead of blocking, and drop stderr so shell noise
    // ("you have mail", instrumentation banners) never reaches the parser.
    let mut child = Command::new(&invocation.program)
        .args(&invocation.leading_args)
        .args(["-lic", "echo $PATH"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .inspect_err(|e| tracing::debug!("Could not run login shell {shell}: {e}"))
        .ok()?;

    let output = match wait_with_timeout(&mut child, SHELL_PATH_TIMEOUT) {
        Some(output) => output,
        None => {
            tracing::warn!("Login shell {shell} did not return a PATH in time; killing it");
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
    };

    if !output.status.success() {
        tracing::debug!("Login shell {shell} exited with {}", output.status);
        return None;
    }

    // An interactive shell may print banners before our echo, so take the
    // last non-empty line rather than the whole of stdout.
    let path = String::from_utf8_lossy(&output.stdout)
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())?
        .to_string();

    // A PATH with no separator and no root-anchored entry is almost certainly
    // profile output we mistook for the answer.
    if !path.contains('/') {
        tracing::debug!("Ignoring implausible PATH from {shell}: {path}");
        return None;
    }

    tracing::info!("Resolved login-shell PATH from {shell}");
    Some(path)
}

#[cfg(not(unix))]
fn login_shell_path() -> Option<String> {
    None
}

#[cfg(unix)]
fn windows_registry_path() -> Option<String> {
    None
}

/// Current machine + user PATH from the registry, followed by any entries of
/// the process PATH not already present.
#[cfg(windows)]
fn windows_registry_path() -> Option<String> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    let read = |root, key: &str| -> Option<String> {
        RegKey::predef(root)
            .open_subkey(key)
            .ok()?
            .get_value::<String, _>("Path")
            .ok()
    };
    let machine = read(
        HKEY_LOCAL_MACHINE,
        r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
    );
    let user = read(HKEY_CURRENT_USER, "Environment");
    if machine.is_none() && user.is_none() {
        return None;
    }
    let mut entries: Vec<String> = Vec::new();
    let process = std::env::var("PATH").unwrap_or_default();
    for part in [
        machine.unwrap_or_default(),
        user.unwrap_or_default(),
        process,
    ]
    .iter()
    .flat_map(|p| p.split(';'))
    .map(|p| expand_windows_env(p.trim()))
    .filter(|p| !p.is_empty())
    {
        if !entries.iter().any(|e| e.eq_ignore_ascii_case(&part)) {
            entries.push(part);
        }
    }
    Some(entries.join(";"))
}

/// Expand `%NAME%` references using the process environment.
#[cfg(any(windows, test))]
fn expand_windows_env(value: &str) -> String {
    let mut out = String::new();
    let mut rest = value;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(v) if !name.is_empty() => out.push_str(&v),
                    _ => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Wait for `child`, giving up after `timeout` and returning its output.
///
/// `std::process::Child` has no timed wait, so poll `try_wait`. The polling
/// interval is short enough to stay responsive and long enough not to spin.
#[cfg(unix)]
fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout: std::time::Duration,
) -> Option<std::process::Output> {
    use std::io::Read;

    let deadline = std::time::Instant::now() + timeout;

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = Vec::new();
                if let Some(mut pipe) = child.stdout.take() {
                    let _ = pipe.read_to_end(&mut stdout);
                }
                return Some(std::process::Output {
                    status,
                    stdout,
                    stderr: Vec::new(),
                });
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Err(_) => return None,
        }
    }
}

/// Directories tools commonly install into that a GUI PATH usually misses.
///
/// Only used after both the process PATH and the login-shell PATH have failed,
/// so this list is a safety net for the case where the shell probe itself
/// could not run (locked-down `$SHELL`, container, sandbox).
fn fallback_bin_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Some(home) = dirs::home_dir() {
        dirs.extend([
            home.join(".local/bin"),
            home.join("bin"),
            home.join(".opencode/bin"),
            home.join(".cargo/bin"),
            home.join(".bun/bin"),
            home.join(".deno/bin"),
            home.join(".volta/bin"),
            home.join(".npm-global/bin"),
            home.join(".yarn/bin"),
            home.join(".claude/local"),
        ]);
    }

    dirs.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/snap/bin"),
        // Homebrew on Linux, MacPorts, Nix profiles.
        PathBuf::from("/home/linuxbrew/.linuxbrew/bin"),
        PathBuf::from("/opt/local/bin"),
        PathBuf::from("/nix/var/nix/profiles/default/bin"),
        PathBuf::from("/run/current-system/sw/bin"),
    ]);

    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".nix-profile/bin"));
    }
    if let Ok(user) = std::env::var("USER") {
        dirs.push(PathBuf::from(format!("/etc/profiles/per-user/{user}/bin")));
    }

    #[cfg(windows)]
    dirs.extend(windows_fallback_dirs());

    dirs
}

/// Where Windows package managers put executables: WinGet's alias links and
/// portable package folders, the Store app-alias folder, `~/.local/bin`
/// (uv, the llama.app installer) and Scoop shims.
#[cfg(windows)]
fn windows_fallback_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let local = PathBuf::from(local);
        dirs.push(local.join(r"Microsoft\WinGet\Links"));
        dirs.push(local.join(r"Microsoft\WindowsApps"));
        // Portable WinGet packages (e.g. ggml.llamacpp) live in per-package
        // folders that WinGet adds to the user PATH.
        if let Ok(entries) = std::fs::read_dir(local.join(r"Microsoft\WinGet\Packages")) {
            dirs.extend(entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
        }
    }
    if let Ok(profile) = std::env::var("USERPROFILE") {
        let profile = PathBuf::from(profile);
        dirs.push(profile.join(r".local\bin"));
        dirs.push(profile.join(r"scoop\shims"));
    }
    dirs
}

/// Locate an executable by name the way the user's terminal would.
///
/// Tries, in order: the process `PATH`, the login-shell `PATH`, then
/// well-known user-local install directories. Returns the resolved absolute
/// path, or `None` if the tool genuinely is not installed.
pub fn find_binary(name: &str) -> Option<PathBuf> {
    // Inside a Flatpak sandbox none of the checks below mean anything: the
    // tool lives on the host filesystem, which we cannot stat. Ask the host to
    // resolve it and trust its answer.
    if crate::sandbox::current().needs_host_proxy() {
        return find_binary_on_host(name);
    }

    if let Ok(path) = which::which(name) {
        return Some(path);
    }

    // Relative PATH entries resolve against cwd; absolute ones (the ones we
    // care about) do not. Root is a harmless stand-in when cwd is unavailable,
    // which can happen in sandboxed GUI contexts.
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));

    if let Some(shell_path) = shell_path() {
        if let Ok(path) = which::which_in(name, Some(&shell_path), &cwd) {
            return Some(path);
        }
    }

    for dir in fallback_bin_dirs() {
        let candidate = dir.join(name);
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let exe = dir.join(format!("{name}.exe"));
            if is_executable_file(&exe) {
                return Some(exe);
            }
        }
    }

    None
}

/// Resolve an executable using the *host's* login shell, for Flatpak.
///
/// Returns the host-side absolute path. It is deliberately not validated
/// against our own filesystem — the whole point is that the path exists out
/// there and not in here — so callers must spawn it through
/// [`crate::sandbox::host_invocation`] rather than executing it directly.
#[cfg(unix)]
fn find_binary_on_host(name: &str) -> Option<PathBuf> {
    use std::process::{Command, Stdio};

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let invocation = crate::sandbox::host_invocation(&shell, [], None);

    // `name` can come from user-authored MCP config, so it is passed as a
    // positional argument rather than interpolated into the script — `$1` is
    // never re-parsed as shell syntax. `--` stops `command` from reading a
    // leading dash as a flag.
    let mut child = Command::new(&invocation.program)
        .args(&invocation.leading_args)
        .args(["-lic", "command -v -- \"$1\"", "lr-probe", name])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .inspect_err(|e| tracing::debug!("Could not probe host for {name}: {e}"))
        .ok()?;

    let output = match wait_with_timeout(&mut child, SHELL_PATH_TIMEOUT) {
        Some(output) => output,
        None => {
            tracing::warn!("Host probe for {name} timed out; killing it");
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
    };

    if !output.status.success() {
        // A non-zero exit is `command -v` reporting "not installed".
        return None;
    }

    // As with the PATH probe, an interactive shell may print banners first, so
    // take the last non-empty line.
    let resolved = String::from_utf8_lossy(&output.stdout)
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())?
        .to_string();

    // `command -v` also succeeds for shell builtins and aliases, which print
    // something that is not a path. Only an absolute path is executable by us.
    if !resolved.starts_with('/') {
        tracing::debug!("Host resolved {name} to a non-path ({resolved}); ignoring");
        return None;
    }

    Some(PathBuf::from(resolved))
}

#[cfg(not(unix))]
fn find_binary_on_host(_name: &str) -> Option<PathBuf> {
    // Flatpak is Linux-only, so this branch is unreachable in practice.
    None
}

/// Whether `path` is something we can actually execute.
///
/// Symlinks are followed (`metadata`, not `symlink_metadata`) because these
/// install dirs are full of them — `~/.local/bin/agy` is typically a link into
/// a versioned directory.
fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };

    if !meta.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_windows_env_references() {
        std::env::set_var("LR_TEST_EXPAND", "C:\\Users\\me");
        assert_eq!(
            expand_windows_env("%LR_TEST_EXPAND%\\bin"),
            "C:\\Users\\me\\bin"
        );
        // Unknown variables and stray percent signs are left as-is.
        assert_eq!(expand_windows_env("%LR_NOPE_X%\\a"), "%LR_NOPE_X%\\a");
        assert_eq!(expand_windows_env("50%"), "50%");
    }

    #[test]
    fn refresh_rebuilds_the_cache() {
        let before = shell_env();
        let after = refresh_shell_env();
        assert_eq!(before.contains_key("PATH"), after.contains_key("PATH"));
        assert_eq!(shell_env(), after);
    }

    #[test]
    fn shell_env_always_yields_a_path() {
        // Either the login shell answered or we fell back to the process
        // PATH; a completely absent PATH would break every lookup.
        let env = shell_env();
        assert!(env.contains_key("PATH"), "shell_env() produced no PATH");
    }

    #[test]
    fn shell_path_is_cached_and_stable() {
        assert_eq!(shell_path(), shell_path());
    }

    #[cfg(unix)]
    #[test]
    fn resolved_path_contains_a_real_directory() {
        let path = shell_path().expect("a PATH should always resolve");
        assert!(
            path.split(':').any(|entry| Path::new(entry).is_dir()),
            "no entry in resolved PATH exists: {path}"
        );
    }

    #[test]
    fn finds_a_binary_that_exists() {
        // `sh` is present on every unix; `cmd` on every Windows.
        let name = if cfg!(unix) { "sh" } else { "cmd" };
        let found = find_binary(name);
        assert!(found.is_some(), "{name} should be locatable");
    }

    #[test]
    fn missing_binary_resolves_to_none() {
        assert!(find_binary("lr-definitely-not-a-real-binary-xyz").is_none());
    }

    #[test]
    fn directories_are_not_mistaken_for_executables() {
        // Guards the is_file() check: /usr/bin is executable-by-mode but is
        // not something we can run.
        assert!(!is_executable_file(Path::new("/usr")));
    }

    #[test]
    fn fallback_dirs_are_absolute() {
        for dir in fallback_bin_dirs() {
            assert!(dir.is_absolute(), "{dir:?} should be absolute");
        }
    }
}
