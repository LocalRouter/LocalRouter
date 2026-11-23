---
name: providers
description: Configure and check providers, then find models.
---

Call `lr_provider_types_list` and `lr_providers_list` to find supported types and current instances. Use `lr_providers_create` for keyless providers. Use the desktop provider dialog for API-key or OAuth setup and keep credentials out of tool arguments. Run `lr_providers_check`, then `lr_models_refresh` for the provider. Page through `lr_models_list` with `next_offset` and inspect a model with `lr_models_get`. Use `lr_providers_test_model` for an explicitly approved inference smoke test. It may incur charges.
