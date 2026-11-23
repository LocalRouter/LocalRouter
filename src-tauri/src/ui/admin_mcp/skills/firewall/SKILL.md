---
name: firewall
description: Inspect and change permissions.
---

Read `lr_firewall_get` for the target client. Specify a patch under `mcp_permissions`, `skills_permissions`, or `model_permissions`. Use values `allow`, `ask`, or `off`; use null to clear an override. Call `lr_firewall_preview` and inspect the affected clients before calling `lr_firewall_apply` with the same changes and returned revision.
