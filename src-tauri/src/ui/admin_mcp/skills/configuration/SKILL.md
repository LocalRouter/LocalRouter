---
name: configuration
description: Read and update supported application preferences.
---

Call `lr_config_list_sections`, then read a section with `lr_config_schema` and `lr_config_get`. Pass only the keys you want to change to `lr_config_preview`. Apply the same patch and revision with `lr_config_apply`, then read back the section. Follow any restart or desktop setup instructions in the result.
