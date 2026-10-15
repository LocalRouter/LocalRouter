# Client privacy and credential controls

Client Settings provides independent local execution, no-retention and metadata-only monitor controls. Defaults preserve existing behavior. Profiles narrow normal model permissions; they do not authorize providers or models.

Local execution requires a LocalRouter-managed embedded process, or a reviewed local-execution declaration on a configured loopback endpoint. An address, provider name or machine on the LAN is not proof of local inference.

No retention requires a reviewed storage/account/inference-chain declaration, including for embedded engines. Local execution alone does not establish storage behavior. Where a gateway has a verified downstream ZDR mapping, LocalRouter also forces ZDR and denies data collection. Unsupported endpoint mappings fail before dispatch. These are user declarations with provenance, not vendor certifications.

Provider Settings stores the source, review date and optional expiry. Evidence is bound to provider type, credentials, endpoint, settings and the application version. Changes invalidate it until reviewed again. Dynamic subscription identities currently have no supported evidence binding. References should contain policy/configuration information, never credentials or prompts.

Restricted profiles support native LLM gateway requests. Raw/reverse proxy, MCP and automatic MCP execution are unavailable. Audio/image/moderation mappings and native gateway decision controls without verified ZDR support are rejected. Active direct safety-model scans are also unsupported under strict profiles: their executors snapshot credentials and do not inherit gateway controls. Requests are rejected rather than skipping their configured safety scan. Decision classifiers use the requesting client's policy.

No retention disables new exact response-cache entries/hits, session affinity/classifier reuse and automatic Responses persistence. Explicit `store:true` or prior-response continuation is rejected. New persistent Files and Batch jobs are rejected. Existing batch jobs can retain accounting/ownership evidence and use supported control/cleanup operations; refresh does not ingest new result payload files under no retention.

Metadata-only monitoring, also implied by no retention, removes bodies, previews, findings, raw traffic and arbitrary errors before storing or emitting a new event. Typed lifecycle, usage and cost metadata remain. Redaction is sticky through completion updates, even if the profile is later disabled. Changing a profile does not rewrite earlier captures or in-flight events created with their earlier policy. Use Monitor Clear and the existing artifact deletion controls deliberately.

Profiles govern new automatic gateway storage. Previously retained files, results and Responses, manually saved Replay datasets/reports and deliberate exports retain their own lifecycle. Application diagnostics, client applications and operating-system logging are separate from monitor capture. Access logging remains metadata-only, disabled by default, with its existing retention configuration.

Credential expiry is an absolute timestamp; requests at or after the deadline fail. Both API-key authentication and existing OAuth tokens check current enabled/expiry state. Idle review is advisory and uses recorded successful authentication; it never silently revokes a key. Revoke disables the client, revokes OAuth tokens and removes its secret while retaining settings and usage. Enabling a revoked client does not restore the removed secret; rotate to issue a replacement. Rotation, disabling and deletion also revoke outstanding OAuth tokens.
