---
name: client-setup
description: Create a client and connect an application.
---

List clients with `lr_clients_list`. Call `lr_clients_create` with a descriptive name and the desired LLM and MCP modes. Approve the change in LocalRouter. Use its returned client ID with `lr_clients_connection`, then open that client’s Connection tab to copy the credential into the application. Load `model-routing` and `firewall` for the next steps.
