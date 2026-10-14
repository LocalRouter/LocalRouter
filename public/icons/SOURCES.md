# Embedded provider logo sources

These original upstream assets are bundled locally for the app and website demo.
They are not fetched at runtime. Brand names and marks belong to their respective
owners. Ollaya and Laya publish under Apache-2.0; stable-diffusion.cpp publishes
under MIT. Their original license files are included alongside the assets.

| Local asset | Official upstream source (pinned revision) |
| --- | --- |
| `ollaya.svg` | [Ollaya website logo](https://github.com/ollaya-dev/ollaya/blob/37fcfa9f8a35b6b389447ffca49e4b4242970f81/site/public/static/logo.svg) |
| `laya.svg` | [Laya logo mark](https://github.com/NandhaKishorM/laya/blob/fa9a2a7070b1789912a49ae24603bbfb1a78b001/assets/logo-mark.svg) |
| `sdcpp.png` | [stable-diffusion.cpp logo](https://github.com/leejet/stable-diffusion.cpp/blob/3f8527a46c54ecf4cb4ed6003da8e8982283c73c/assets/logo.png) |

Assets are copied without modification. `llamacpp_embedded` shares the existing
`llamacpp.png` asset with the llama.cpp server provider.

As of 2026-10-02, no project logo was found in the official repositories or
READMEs for [Kev](https://github.com/jaredpalmer/kev),
[Von](https://github.com/wfzyx/von), or [Decider](https://github.com/Mapika/decider).
They use the standard provider category icon. Kev's playground favicon is the
Vercel starter mark, so it is not used as Kev branding.

## Atlassian and Datadog MCP templates

- `atlassian.svg`: official Atlassian logo, https://atlassian.github.io/atlassian-mcp-server/images/atlassian_logo_brand_RGB.svg (upstream https://github.com/atlassian/atlassian-mcp-server). Atlassian trademark; used solely to identify the service, subject to https://www.atlassian.com/legal/trademark .
- `datadog.png`: official logo from Datadog documentation, https://docs.dd-static.net/img/dd-logo-n-200.png (linked by https://docs.datadoghq.com/mcp_server/setup/). Datadog trademark; used solely to identify the service, subject to https://www.datadoghq.com/legal/terms/ .

## Pi client template

- `pi.svg`: official Pi logo, https://pi.dev/logo.svg (byte-identical to https://pi.dev/logo-auto.svg, which the upstream README at https://github.com/earendil-works/pi embeds; retrieved 2026-10-10, SHA-256 `abd66e7868b2d24f0f0895f9237ee8a6dcb22337583b0dc54aeb595acecb4d6b`). Copied without modification. The Pi source code is MIT-licensed; the logo is not part of that repository and no separate logo license is published. Pi mark of its owners; used solely to identify the app.

## FastFlowLM provider

- `fastflowlm.png`: official FastFlowLM logo, [`assets/logo.png`](https://github.com/ROCm/FastFlowLM/blob/1405461da9328e382a8ccf408c1250049598bcc1/assets/logo.png) in the upstream repository (the image its README shows; retrieved 2026-10-11, SHA-256 `928588533d5e91186c855291a6805660a0ba986a0be87318803083cd62c2b47d`). Copied without modification. The repository is MIT-licensed; its license (`LICENSE_RUNTIME.txt` at revision `e5e33d9f64388f4e239eb6584493c76ef5a07231`) is included as `fastflowlm-LICENSE.txt`. FastFlowLM name and mark of their owners; used solely to identify the provider.

## LLM Gateway provider

- `llmgateway.svg`: official black symbol from LLM Gateway's [brand assets](https://llmgateway.io/brand), copied without modification from [`apps/ui/public/brand/logo-black.svg`](https://github.com/theopenco/llmgateway/blob/ba743696ea6db4b7a68ba764ddd70edc19fbb95d/apps/ui/public/brand/logo-black.svg) (retrieved 2026-10-14). The upstream repository licenses this content under AGPLv3; its [license notice](https://github.com/theopenco/llmgateway/blob/ba743696ea6db4b7a68ba764ddd70edc19fbb95d/LICENSE) is included as `llmgateway-LICENSE.txt`. Copyright (c) 2025-present LLMGateway, Inc. The name and mark belong to their owners and identify the provider without implying endorsement. The existing icon container supplies a light background in dark mode.
