# Usage polling: back-off, traffic accounting, defaults on upgrade

Pre-release review of the subscription usage tracking feature (12451826)
before 0.0.154.

## Problems

1. `poll_credits` returned without recording an attempt when
   `check_credits()` gave `None`. OpenRouter returns `None` for any failure
   (401 revoked key, 429, offline), so `/auth/key` was requested on every
   30 s scheduler tick (~2,900 requests/day per instance), ignoring 429.
2. The credits request went through the provider's middleware client, so
   `UsageObserverMiddleware` counted the poll's own response as account
   traffic and the account was polled at the active rate (5 min) forever
   instead of hourly.
3. "Ask connected providers" defaults on, so upgrading would start
   background requests to ChatGPT, Copilot and OpenRouter usage endpoints
   with no user action, against the privacy policy (network only on user
   action or update checks; provider health checks are off by default).

## Decisions

- (1) A `None` from `check_credits` defers the source by the failure back-off
  (5 min doubling to 1 h) without a status row; a success resets it.
- (2) Poll requests carry the `lr_providers::http_client::UsagePoll`
  extension; the usage observer skips them.
- (3) Per the user: off for existing installations, on for new installations
  and providers added later. `usage_tracking.poll_excluded_providers` lists
  provider instances that are not queried. Config v31 migration adds every
  provider that exists at upgrade; new installs start at v31 with an empty
  list. Settings → Usage lists ChatGPT Plus/Pro, Copilot and OpenRouter
  providers with a switch each. Renaming a provider renames its entry.

## Verification

- `migration::tests::test_migrate_to_v31_excludes_existing_providers_from_usage_polling`
- `ui::usage_poller::tests::missing_credits_back_off_instead_of_retrying_every_tick`
- Workspace clippy `-D warnings`, fmt, `tsc --noEmit`; tests of lr-config,
  lr-usage, lr-providers and the app crate.

## Final steps

- [x] Plan review: poller filters excluded instances for all three sources;
  changing the list triggers an immediate poll; TS type and demo mock updated.
- [x] Test coverage review: migration and back-off unit-tested; the observer
  skip is a two-line guard on the request extension.
- [x] Bug hunt: a provider deleted while excluded leaves a stale name in the
  list (harmless; re-adding it under the same name stays excluded).
- [x] Commit and push.
