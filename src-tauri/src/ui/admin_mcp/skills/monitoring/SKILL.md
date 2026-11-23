---
name: monitoring
description: Inspect LocalRouter traffic and retention.
---

Start with `lr_monitor_stats`. Call `lr_monitor_events_list` with the target client ID and use the returned `before_sequence` for the next page. Call `lr_monitor_event_get` for an event. Results contain traffic metadata. Load `active-intercept` to inspect requests currently held for approval.
