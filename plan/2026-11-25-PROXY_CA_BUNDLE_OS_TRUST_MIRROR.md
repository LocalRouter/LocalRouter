# Proxy CA bundle that mirrors the OS trust store

## Problem

An agent running in Codex (configured for the HTTPS inspection proxy) ran
`gh issue view …` and got `tls: failed to verify certificate: x509:
certificate signed by unknown authority` for `api.github.com`.

`api.github.com` is not intercepted — the proxy blind-tunnels it. The cause
was the CA variable: permanent Codex setup wrote
`SSL_CERT_FILE=<LocalRouter root CA>` into `~/.codex/.env`. Codex exports
that into its process environment, every command it runs inherits it, and
OpenSSL, curl, Python and Go treat `SSL_CERT_FILE` as the **entire** trust
store. Every non-intercepted host failed for those commands. Reproduced with
`SSL_CERT_FILE=root-ca.pem gh api /zen` (and curl, error 60). The generic
"custom tool" instructions had the same flaw.

## Approach

Point every `SSL_CERT_FILE`-style variable at a combined bundle: the roots the
OS trusts for TLS, plus our CA, kept in step with the OS.

A file cannot defer to the OS at connection time (each tool parses it
itself), so "proxying to the OS" is approximated by mirroring: re-read the OS
trust decisions every 60 s and atomically rewrite the file when they change.

### OS trust, per platform (`crates/lr-truststore`)

- **macOS**: trust settings in the user, admin and system domains, with
  Apple's precedence (user > admin > system). Entries are evaluated directly
  (`SecTrustSettingsCopyTrustSettings`) rather than via
  `security-framework`'s helper, because a bundle can only express "trusted
  for every host and program": trust limited to a host
  (`kSecTrustSettingsPolicyString`), an application, or an allowed error grants
  nothing; distrust (`Deny`) for TLS is honored however narrowly scoped.
  Settings for non-TLS policies do not decide.
- **Windows**: CurrentUser `Root` (includes machine and Group Policy roots),
  minus the `Disallowed` store, minus certs whose enabled purposes exclude
  server auth, minus certs outside their validity period.
- **Linux/Unix**: the distribution's compiled bundle (`update-ca-trust` /
  `update-ca-certificates` output), which already applies the distro and admin
  blocklists. Fedora's TLS-only extract is preferred.
- `SSL_CERT_FILE`/`SSL_CERT_DIR` in LocalRouter's own environment are ignored
  (they may point at this very bundle).

### Limits (documented, not fixable in a bundle)

- Distrust of an **intermediate** or leaf (e.g. the admin-domain `Deny`
  entries for "EM … TLS CA" intermediates on this machine) cannot be expressed
  in a root bundle; OS-native verifiers enforce it, OpenSSL/Go-with-a-file do
  not.
- Online revocation (OCSP/CRL) is up to each tool.
- Windows downloads some Microsoft-program roots lazily; one never needed on
  the machine is absent until Windows fetches it.
- Processes read the file at start; a running process keeps its old view.

### Bundle maintenance (`src-tauri/src/launcher/ca_bundle.rs`)

- Header + sorted, deduplicated OS roots + our CA last (once, even if the OS
  also trusts it). Deterministic, so unchanged trust never rewrites the file.
- Atomic replace (temp file + rename; in-place fallback on Windows).
- An unreadable or empty OS store keeps the existing file (never narrows
  trust).
- `spawn_refresher` (started next to the proxy client sync) refreshes every
  60 s, only while the bundle exists; the first pass runs at startup and
  upgrades older (certifi-based) bundles.
- `ensure_combined_bundle` creates it on demand (setup UI, Configure, sync).

### Tool plans (`proxy_setup.rs`)

- `ProxyPlan.ca_bundle`: the bundle the plan points the tool at.
  `ProxyPlan.ca_env_value`: what to assign to `ca_env_var` when not the bare
  CA.
- Codex: `.env` `SSL_CERT_FILE` → bundle. The one-off keeps
  `CODEX_CA_CERTIFICATE=<root CA>` (Codex-only, not used by children).
- Aider: unchanged target (bundle), now OS-mirrored instead of certifi-based.
- Generic tools: `SSL_CERT_FILE` → bundle (UI shows `ca_env_path`).
- If the bundle cannot be built, automatic setup is withdrawn rather than
  writing a config naming a missing file.

### Migration (`proxy_sync.rs`)

- `FileRecord.ca_bundle` (serde default). A configured file whose record (or,
  without a record, whose content) does not match the plan's bundle is
  rewritten once, so existing Codex configs move off the bare CA at the next
  startup.
- Orphan detection also recognizes the bundle path.

## Testing

- Unit: trust-entry decisions (constrained trust, deny, other policies,
  unspecified), PEM round trip, bundle composition/dedupe, rewrite-only-on-
  change, distrust/addition reflected, unreadable store keeps file, no temp
  file left, legacy bundle detection, `SSL_CERT_FILE` never names the bare CA
  (Codex/Aider/generic), Codex config migration (with and without a record).
- Cross-platform: `cargo clippy -p lr-truststore` for Windows MSVC and Linux.
- Live (debug app, `~/.localrouter-dev`): legacy bundle upgraded at startup;
  158 certs = 157 TLS-trusted system roots + our CA; Apple "Developer ID"
  CA (not TLS-trusted) excluded; host-constrained user cert excluded; denied
  admin certs absent. `gh`, curl, Python, git and Go succeed with the bundle
  against public hosts and against a host whose cert chains to our CA.
  Tampering with the file is repaired by the refresher within one interval.

## Final steps

- [x] Plan review against implementation
- [x] Test coverage review
- [x] Bug hunt
- [x] Commit and push
