//! Building child-process commands that see the user's tools.

use std::path::Path;

/// A `tokio` command for `program` that runs with the user's login-shell
/// PATH and, inside a Flatpak sandbox, on the host (where the user's tools
/// live). Environment variables must be passed here rather than set on the
/// returned command, because under Flatpak they have to be forwarded to the
/// host process explicitly.
pub fn host_command(
    program: &Path,
    args: impl IntoIterator<Item = String>,
    env: Vec<(String, String)>,
) -> tokio::process::Command {
    let mut all_env: Vec<(String, String)> = lr_utils::binary::shell_env().into_iter().collect();
    all_env.extend(env);
    let invocation = lr_utils::sandbox::host_invocation(&program.to_string_lossy(), all_env, None);
    let mut cmd = tokio::process::Command::new(&invocation.program);
    cmd.args(&invocation.leading_args).args(args);
    for (k, v) in &invocation.envs {
        cmd.env(k, v);
    }
    #[cfg(windows)]
    {
        // Don't flash a console window for background engines.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}
