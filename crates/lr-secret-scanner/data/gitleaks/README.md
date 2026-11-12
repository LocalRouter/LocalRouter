# Embedded Gitleaks rules

The scanner bundles the MIT-licensed Gitleaks v8.30.1 default configuration,
commit `83d9cd684c87d95d656c1458ef04895a7f1cbd8e`:
https://github.com/gitleaks/gitleaks/tree/83d9cd684c87d95d656c1458ef04895a7f1cbd8e

`gitleaks.toml` and `LICENSE` are unmodified upstream files. `upstream.json`
records their checksums. `rules.json` is the reproducible content-only snapshot
used by the Rust scanner, including the upstream copyright and license notice. No executable, runtime downloads, telemetry, credential
verification or network calls are involved. Compiled patterns and keyword
acceleration are shared across scanner instances.

201 upstream rules are embedded. The snapshot records a reason for each of the
21 excluded rules: LocalRouter overrides for existing formats, tuned generic
assignment/private-key/JWT detection, public identifiers/validation keys, and
rules requiring file-path metadata unavailable in model-bound text. An unavailable
file-path or commit predicate is never silently treated as a match. Content
allowlists preserve OR/AND and secret/match/line semantics. Stopwords target the
captured secret, not arbitrary nearby text. `gitleaks:allow` comments are not a
runtime bypass: decisions come from LocalRouter's authenticated client policy.

Gitleaks' default first-nonempty-capture selection and per-rule entropy thresholds
are preserved. The generator adapts Go/RE2 ASCII character classes/boundaries
and literal braces for Rust regex. The boolean-placeholder allowlist is anchored
as a complete alternation so words embedded inside a real token are not allowed. Encoded JWT header-classification captures
are not credentials: LocalRouter captures the whole encoded token and validates
its decoded header and signature. Curl authentication rules use decoded-password
and generic-placeholder checks, as do LocalRouter's standalone headers.

To reproduce or verify the snapshot (Python 3.11+):

```
python3 scripts/update-secret-rules.py
python3 scripts/update-secret-rules.py --check
```

To update, choose an upstream release and resolve it to an immutable commit.
Download `config/gitleaks.toml` and `LICENSE` from that commit, record the version,
commit and SHA-256 checksums in `upstream.json`, regenerate, review the exclusions,
and run the importer tests and Rust detection quality suite. New source fields
fail generation until their semantics are reviewed. Never refresh at runtime.
