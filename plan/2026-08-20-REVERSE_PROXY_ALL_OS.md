# Reverse proxy: provider relocation on every OS

Follow-up to `2026-08-20-REVERSE_PROXY_MODE.md`. The proxy itself is portable;
only the *relocation plans* in `src-tauri/src/launcher/reverse_setup.rs` were
macOS-first. This plan makes every provider do the best it can on macOS,
Windows and Linux, and makes the executor able to run those plans.

## Checklist

- [x] `Cmd` gains `env` (set/unset vars for the child) and `detach` (spawn a
      GUI app without waiting for it to exit); `run_one` honours both,
      `display()` shows the env prefix.
- [x] `ReversePlan.configure_restarts`: the configure phase already restarted
      the provider (single privileged transaction on Linux); `supports_auto`
      accepts it in place of `start`.
- [x] Ollama / Windows: `setx` + graceful `taskkill` + forced `taskkill` +
      detached relaunch of `ollama app.exe` with `OLLAMA_HOST` set explicitly;
      undo `reg delete HKCU\Environment /v OLLAMA_HOST /f` + relaunch with the
      var removed. Manual fallback when the app exe isn't found.
- [x] Ollama / Linux: when `ollama.service` exists and `pkexec` is available,
      one `pkexec sh -c` writes `/etc/systemd/system/ollama.service.d/localrouter.conf`,
      `daemon-reload`s and restarts; undo removes the drop-in the same way.
      Manual + one-off fallback otherwise.
- [x] One-off commands shell-correct per OS (PowerShell `$env:` on Windows).
- [x] LM Studio: also look in `~/.cache/lm-studio/bin` for `lms`.
- [x] Tests per OS (`cfg(target_os)`), and cross-OS invariants (every
      automatable plan is reversible, stop is best-effort, nobody is left
      with a blank panel).
- [x] Update the previous plan's "reliable automation" paragraph.
- [x] Final: plan review, coverage review, bug hunt, clippy/fmt/test, commit.

## Per provider × OS

| Provider | macOS | Windows | Linux |
|---|---|---|---|
| Ollama | auto (launchctl) | auto (setx / taskkill / relaunch) | auto via systemd + pkexec, else manual |
| LM Studio | auto via `lms` | auto via `lms` | auto via `lms` |
| Jan | manual (GUI) | manual | manual |
| GPT4All | manual (GUI) | manual | manual |
| LocalAI | manual + one-off | manual + one-off | manual + one-off |
| llama.cpp | manual + one-off | manual + one-off | manual + one-off |
| custom | manual | manual | manual |

Jan and GPT4All keep GUI instructions on every OS: their settings stores
(Jan's JSON, GPT4All's QSettings) are not documented stably enough to edit
blind, and a wrong edit would corrupt a user's app config.

## Why the executor changes

- **Environment.** `setx` writes the registry; a process spawned by LocalRouter
  inherits LocalRouter's environment, not the registry. Relaunching Ollama
  from here without passing `OLLAMA_HOST` explicitly would start it on the
  old port and the relocation would verify-fail. Undo has the mirror problem
  if LocalRouter itself was started after the var was set.
- **Detach.** `open -a` returns immediately; `"ollama app.exe"` does not —
  `Command::output()` would block until the user quits Ollama.
- **One privileged prompt on Linux.** `pkexec` does not cache credentials, so
  configure / stop / start as three commands means three password dialogs.
  One transaction that also restarts, flagged with `configure_restarts`, is
  one prompt.

## Bug hunt findings (fixed before commit)

- Undo reused `start`, so on Windows the relaunch would have carried
  `OLLAMA_HOST=<new port>` and moved nothing. Added `undo_start` and
  `ReversePlan::into_undo()`; the undo command now feeds the flipped plan to
  `relocate`.
- A `configure_restarts` plan skipped the "old port released" check, so a
  slow systemd stop would have been blamed on the listener bind. `relocate`
  now waits for the old port after such a configure.

## Not verified on real hardware

The Windows and Linux branches are exercised only by `cfg(target_os)` unit
tests in CI and by the cross-OS invariants; nobody has clicked through them
on a real machine yet.

## Final steps

1. Plan review against implementation.
2. Test coverage review.
3. Bug hunt.
4. Commit (own files only).
