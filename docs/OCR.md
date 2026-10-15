# Explicit OCR

Enable **Client Settings → Document operations → Allow OCR** for a client.
This permission defaults off and is independent of web search, MCP tools and
model permissions. The current client credential and explicit model permission
are checked before dispatch.

`POST /v1/ocr` (also `/ocr`) accepts a bounded inline document and selected pages:

```json
{
  "model": "mistral/mistral-ocr-latest",
  "filename": "source.pdf",
  "document": {
    "type": "document_url",
    "document_url": "data:application/pdf;base64,..."
  },
  "pages": [0, 2, 4],
  "include_image_base64": false,
  "image_limit": 0
}
```

The model prefix is the configured LocalRouter provider instance. Direct
Mistral supports explicit `mistral-ocr-*` model IDs; LLM Gateway currently maps
`gateway/mistral-ocr-latest`. Other provider protocols reject OCR. OCR is a
separate endpoint and never an implicit native-document parser choice.

Pages are required: select 1–20 unique zero-based indices between 0 and 9999,
or an inclusive bounded range such as `"0-4"`. Page 0 is the first page.
An omitted/empty page selection, duplicates, reversed/oversized ranges and
automatic model selection fail before IO. This limits the requested work;
a provider that reports work beyond that selection causes an error, with
unknown charges retained rather than an assertion that nothing was billed.

One inline PDF, PNG or JPEG is supported, up to 4 MiB decoded. Images use
`{"type":"image_url","image_url":"data:image/png;base64,..."}` and require
`pages:[0]`. Remote URLs, unowned file IDs, annotation-generation options and
unknown fields have no enabled mapping. `filename` is local provenance and
is not forwarded as an unsupported provider field. Images returned by OCR
remain data: LocalRouter does not fetch or render their remote references.

Normal credential lifetime, model permission, rate limits and sequential
approval checks apply. Restricted local-only/no-retention profiles, free-only
strategies and cost ceilings reject these unverified native operations.
Opaque input cannot satisfy blocking secret/content scans; supply extracted
UTF-8 text through the local document workflow instead. Automatic MCP loops,
classification, compression, affinity, caching and automatic replay are not
part of OCR. A selected file stays local until the explicit extraction action.

The transport performs one dispatch with redirects and fallback disabled.
Compatible SDK errors include `x-should-retry:false`; Try It Out also disables
SDK retries. Cancelling a local request cannot guarantee the upstream stopped
processing or billing. Dispatched operations consume one request quota even
if cancelled or their outcome becomes unknown.

Responses preserve native page indices, markdown, dimensions and
`usage_info.pages_processed`. `extensions.localrouter.ocr` contains the original
filename/MIME/byte count/SHA-256, requested pages and page-based usage. Full
fees remain `null`/`unknown` when not reported; no text tokens or public-rate-card
fees are invented. Numeric aggregate metrics contain only known subtotals.
Extracted markdown is untrusted text, displayed without executing HTML or
loading referenced images. The website demo uses labeled bundled page fixtures;
it does not send or analyze the selected file.

Verified contracts:
- https://docs.mistral.ai/api/endpoint/ocr
- https://docs.llmgateway.io/features/ocr
- https://docs.llmgateway.io/v1_ocr
