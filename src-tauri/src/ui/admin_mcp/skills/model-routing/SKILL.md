---
name: model-routing
description: Inspect and edit model routing.
---

Find the client with `lr_clients_list` after loading client-setup. Use `lr_routing_strategies_get` to inspect the strategy and its client bindings. Inspect the current effective route with `lr_routing_preview`. Use `lr_routing_strategies_manage` with action `update` and a patch to `auto_config`, `rate_limits`, or the free-tier settings. Re-read the strategy and preview the client again. A classifier may still be required on a real request.
