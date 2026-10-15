# Documents

Chat Completions accepts user-message file content parts:

```json
{
  "model": "gateway/google/gemini-model",
  "messages": [{"role": "user", "content": [
    {"type": "text", "text": "Summarize the attachment."},
    {"type": "file", "file": {"filename": "source.pdf", "file_data": "data:application/pdf;base64,..."}}
  ]}],
  "extensions": {"localrouter.documents": {"mode": "native"}}
}
```

Responses accepts the corresponding `{ "type": "input_file", "filename":
"source.pdf", "file_data": "data:application/pdf;base64,..." }` in a user
message's content array. Native files require `store:false` and no
`previous_response_id`. Omitted `store` becomes false for native files.

Native mode preserves the document bytes. OpenRouter explicitly selects
`file-parser` with `pdf.engine:native`; an unsupported model must fail rather
than implicitly choosing paid OCR. LLM Gateway supports explicitly named
`google/gemini-*` and `google-vertex/gemini-*` models. Model-specific MIME and
context limits still apply. Other Chat adapters reject native file input until
their conversion is verified. Existing native Anthropic Messages document
blocks keep their separate validated protocol.

Limits: four files, 4 MiB each, 8 MiB total decoded input. Text files require
valid UTF-8 and at most 512 KiB each. MIME types are PDF and plain/markdown/CSV/
HTML/CSS/JavaScript/XML text. OpenRouter's initial native mapping accepts PDF;
use text mode for other text formats. Unowned file IDs, remote URLs, unknown
fields and file parts outside user messages are rejected. LocalRouter does
not fetch documents or auto-install an extractor.

For local extraction, run a configured local MCP extraction tool and attach its
UTF-8 output with `localrouter.documents.mode:"text"`. Text mode decodes the
attachment before the normal scans/routing pipeline. It records a filename,
MIME type and SHA-256 around the actual text. It does not invent page numbers.
PDF binaries require extraction first; choosing text mode is not a PDF parser.

Native document calls require an explicit model. Compression, text-only
classification, automatic MCP orchestration, duplicate hops, response caching,
session affinity and automatic replay are disabled for opaque files. Native
calls disable gateway/provider fallback and redirects. Normal model permissions,
credential expiry, privacy and rate checks apply. Cost ceilings and free-only
strategies require extracted text because native PDF input cost cannot be
reliably bounded before dispatch. Blocking secret scans or active request
guardrails also require extracted text rather than pretending to inspect PDF
bytes. Notify-mode scanning covers only inspectable text/metadata.

`extensions.localrouter.document_processing` reports source hashes, filenames,
byte counts, processing mode, scan coverage and whether native input usage was
reported. Page count is unknown unless an explicit extractor/OCR supplies it.
Streams retain this metadata; missing usage or cancellation leaves full document
cost unknown. Numeric aggregate metrics include only known cost subtotals.
No parser fee or zero-cost claim is fabricated.

Try It Out → Documents supports one local file and sends it only on pressing
Send document. Responses are shown as untrusted JSON. Cancellation aborts the
local request but does not promise the upstream stopped billing.

Verified contracts:
- https://openrouter.ai/docs/guides/overview/multimodal/pdfs
- https://docs.llmgateway.io/features/documents

Explicit OCR is a separate default-off operation; see [OCR](OCR.md) for page
selection and native protocol limits. This native document path never silently
selects a paid OCR engine.
