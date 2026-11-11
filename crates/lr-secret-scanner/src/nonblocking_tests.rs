use crate::{
    placeholders::Vault,
    wire::{restore_stream, StreamMode, LIMIT},
};
use futures::StreamExt;
use lr_api_keys::{keychain_trait::ResilientSecretStorage, KeychainStorage};
use lr_types::{secret_context::SecretContext, AppError, AppResult};
use serde_json::json;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

struct Broken;
impl KeychainStorage for Broken {
    fn store(&self, _: &str, _: &str, _: &str) -> AppResult<()> {
        Err(AppError::Storage("fixture unavailable".into()))
    }
    fn get(&self, _: &str, _: &str) -> AppResult<Option<String>> {
        Err(AppError::Storage("fixture unavailable".into()))
    }
    fn delete(&self, _: &str, _: &str) -> AppResult<()> {
        Err(AppError::Storage("fixture unavailable".into()))
    }
}
fn vault(owner: &str) -> Arc<Vault> {
    Arc::new(Vault::for_client(
        Arc::new(lr_api_keys::MockKeychain::new()),
        owner,
    ))
}
fn warnings(vault: &Arc<Vault>) -> Arc<Mutex<Vec<String>>> {
    let warnings = Arc::new(Mutex::new(Vec::new()));
    let sink = warnings.clone();
    let weak = Arc::downgrade(vault);
    vault.set_operation_observer(Arc::new(move |event| {
        // Diagnostic masking can reacquire the store; callbacks must not run
        // while the protection operation still owns its storage lock.
        if let Some(vault) = weak.upgrade() {
            let _ = vault.redact_monitor(&mut json!({"message":event.location}));
        }
        if event.action.ends_with("warning") {
            sink.lock().unwrap().push(event.location);
        }
    }));
    warnings
}
#[test]
fn ordinary_mixed_code_with_a_saved_mapping_never_rejects_an_unrelated_request() {
    let vault = vault("ordinary-mixed-code");
    vault
        .register("synthetic-original-absent-from-request")
        .unwrap();
    let notices = warnings(&vault);
    let text = r#"Example: {"code":"print(\"hello\\nworld\")", "html":"&amp;", "percent":"%41"}"#;
    let mut request = json!({"input":[{"role":"user","content":text}]});
    let original = request.clone();
    vault.protect(&mut request).unwrap();
    assert_eq!(request, original);
    assert!(notices.lock().unwrap().is_empty());
}
#[test]
fn storage_outage_uses_a_shared_owned_memory_map_without_durable_writes() {
    let storage: Arc<dyn KeychainStorage> = Arc::new(ResilientSecretStorage::new(Arc::new(Broken)));
    let first = Arc::new(Vault::for_client(storage.clone(), "memory-client-a"));
    let notices = warnings(&first);
    let secret = "synthetic-memory-original\"quoted\"\\path\nsecond line";
    first.register(secret).unwrap();
    let mut request = json!({"input":secret});
    first.protect(&mut request).unwrap();
    let tag = request["input"].as_str().unwrap();
    assert_ne!(tag, secret);
    assert!(storage.persistence_warning());
    let second = Vault::find_client(
        storage.clone(),
        "memory-client-a",
        crate::placeholders::RETENTION_SECS,
    )
    .unwrap()
    .unwrap();
    assert_eq!(second.restore_text(tag, true).unwrap(), secret);
    let foreign = Vault::for_client(storage.clone(), "memory-client-b");
    assert_eq!(foreign.restore_text(tag, true).unwrap(), tag);
    let id = Vault::list_client(storage.clone(), "memory-client-a").unwrap()[0]
        .reference_id
        .clone();
    Vault::purge_client(storage.clone(), "memory-client-a", Some(&id)).unwrap();
    assert_eq!(second.restore_text(tag, true).unwrap(), tag);
    assert!(!notices.lock().unwrap().is_empty());
}
#[test]
fn unreadable_corrupt_and_revoked_stores_preserve_the_request() {
    let broken = Vault::for_client(Arc::new(Broken), "broken-map");
    let mut value = json!({"content":"original request"});
    let original = value.clone();
    broken.register("original request").unwrap();
    broken.protect(&mut value).unwrap();
    assert_eq!(value, original);
    let store = Arc::new(lr_api_keys::MockKeychain::new());
    let active = Vault::for_client(store.clone(), "revoked-map");
    active.register("synthetic revoked original").unwrap();
    Vault::purge_client(store, "revoked-map", None).unwrap();
    let mut value = json!({"content":"synthetic revoked original"});
    let original = value.clone();
    active.protect(&mut value).unwrap();
    assert_eq!(value, original);
}
#[test]
fn signed_content_is_untouched_while_other_text_is_masked() {
    let vault = vault("signed-availability");
    let secret = "synthetic-signed-original-123";
    vault.register(secret).unwrap();
    let notices = warnings(&vault);
    let signed = json!({"type":"thinking","signature":"provider-signature","thinking":secret});
    let mut value = json!({"messages":[{"role":"assistant","content":[signed.clone(),{"type":"text","text":secret}]}]});
    vault.protect(&mut value).unwrap();
    assert_eq!(value["messages"][0]["content"][0], signed);
    assert_ne!(value["messages"][0]["content"][1]["text"], secret);
    assert!(!notices.lock().unwrap().is_empty());
    let mut responses =
        json!({"model":"model","input":[signed.clone(),{"role":"user","content":secret}]});
    vault.protect(&mut responses).unwrap();
    assert_eq!(responses["input"][0], signed);
    assert_ne!(responses["input"][1]["content"], secret);
}
async fn restored(chunks: Vec<Vec<u8>>, mode: StreamMode) -> Vec<u8> {
    let vault = vault("stream-availability");
    let _notices = warnings(&vault);
    let done = Arc::new(AtomicBool::new(false));
    let source = futures::stream::iter(
        chunks
            .into_iter()
            .map(|v| Ok::<_, std::io::Error>(bytes::Bytes::from(v))),
    );
    let mut output = restore_stream(source, vault, mode, done.clone());
    let mut bytes = Vec::new();
    while let Some(chunk) = output.next().await {
        bytes.extend_from_slice(&chunk.unwrap());
    }
    assert!(done.load(Ordering::Acquire));
    bytes
}
#[tokio::test]
async fn invalid_bytes_after_buffered_arguments_are_replayed_once_in_order() {
    let first = format!(
        "data: {}\r\n\r\n",
        json!({"type":"response.function_call_arguments.delta","item_id":"call","delta":"{\"input\":\"<localrouter-redacted"})
    );
    let second = vec![0xff, b'\n', b'\n'];
    let expected = [first.as_bytes(), second.as_slice()].concat();
    assert_eq!(
        restored(vec![first.into_bytes(), second], StreamMode::Sse).await,
        expected
    );
}
#[tokio::test]
async fn incomplete_ndjson_and_sse_are_delivered_exactly() {
    for (mode, wire) in [
        (
            StreamMode::Ndjson,
            b"{\"response\":\"<localrouter-secret".to_vec(),
        ),
        (StreamMode::Sse, b"data: {\"type\":\"unfinished\"".to_vec()),
    ] {
        assert_eq!(restored(vec![wire.clone()], mode).await, wire);
    }
}
#[tokio::test]
async fn oversized_json_and_frame_are_passed_through_without_secret_errors() {
    let bytes = vec![b'x'; LIMIT + 1];
    for mode in [StreamMode::Json, StreamMode::Sse, StreamMode::Ndjson] {
        assert_eq!(restored(vec![bytes.clone()], mode).await, bytes);
    }
}

#[test]
fn protocol_identities_and_keys_remain_valid_when_they_overlap_a_known_original() {
    let vault = vault("protocol-identities");
    vault.register("model").unwrap();
    let notices = warnings(&vault);
    let mut request = json!({"model":"model","messages":[{"role":"user","content":"model"}],"tools":[{"type":"function","name":"model","parameters":{"type":"object"}}]});
    vault.protect(&mut request).unwrap();
    assert_eq!(request["model"], "model");
    assert_eq!(request["tools"][0]["name"], "model");
    assert_ne!(request["messages"][0]["content"], "model");
    assert!(!notices.lock().unwrap().is_empty());
}

#[test]
fn schema_properties_named_like_protocol_fields_are_still_masked() {
    let vault = vault("schema-field-identities");
    let secret = "synthetic-schema-original-123456";
    vault.register(secret).unwrap();
    let mut properties = serde_json::Map::new();
    for key in ["name", "parameters", "model", "role", "object", "choices"] {
        properties.insert(key.into(), json!({"type":"string", "const":secret}));
    }
    let mut request = json!({"model":"provider-model","tools":[{"type":"function","name":"provider-tool","parameters":{"type":"object","properties":properties}}]});
    vault.protect(&mut request).unwrap();
    assert_eq!(request["model"], "provider-model");
    assert_eq!(request["tools"][0]["name"], "provider-tool");
    for value in request["tools"][0]["parameters"]["properties"]
        .as_object()
        .unwrap()
        .values()
    {
        assert_ne!(value["const"], secret);
        assert_eq!(
            vault
                .restore_text(value["const"].as_str().unwrap(), true)
                .unwrap(),
            secret
        );
    }
}

#[test]
fn marker_documentation_is_not_a_missing_secret() {
    let vault = vault("marker-docs");
    let notices = warnings(&vault);
    for text in [
        "localrouter-secret",
        "<localrouter-secret",
        "<localrouter-redacted id='example'/>",
        "&lt;localrouter-secret id='abc123'/&gt;",
        "Discuss \\u006cocalrouter-secret and escaping",
    ] {
        assert_eq!(vault.restore_text(text, false).unwrap(), text);
    }
    assert!(notices.lock().unwrap().is_empty());
    let tag = "<localrouter-secret id='00112233445566778899aabbccddeeff'/>";
    assert_eq!(vault.restore_text(tag, false).unwrap(), tag);
    assert_eq!(notices.lock().unwrap().len(), 1);
}

#[test]
fn large_history_does_not_spend_the_decode_budget_on_duplicate_views() {
    let vault = vault("large-mixed-history");
    let secret = "synthetic-large-history-value-873514";
    vault.register(secret).unwrap();
    let notices = warnings(&vault);
    let example =
        r#"Example: {"code":"print(\"hello\\nworld\")", "html":"&amp;", "percent":"%41"}"#;
    let mut request = json!({"input":(0..900).map(|i|json!({"role":"user","content":format!("{i} {} {secret}", example.repeat(30))})).collect::<Vec<_>>()});
    vault.protect(&mut request).unwrap();
    for item in request["input"].as_array().unwrap() {
        assert!(!item["content"].as_str().unwrap().contains(secret));
    }
    assert!(
        notices.lock().unwrap().is_empty(),
        "{:?}",
        notices.lock().unwrap()
    );
}

#[tokio::test]
async fn incorrectly_labeled_sse_delivers_first_frame_before_eof() {
    use futures::FutureExt;
    let vault = vault("sniff-stream");
    let notices = warnings(&vault);
    for prefix in [
        "event: response.created\ndata: {\"type\":\"response.created\"}\n\n",
        ": ping\n\n",
    ] {
        let source =
            futures::stream::once(
                async move { Ok::<_, std::io::Error>(bytes::Bytes::from(prefix)) },
            )
            .chain(futures::stream::pending());
        let mut output = restore_stream(
            source,
            vault.clone(),
            StreamMode::Json,
            Arc::new(AtomicBool::new(false)),
        );
        assert!(
            output.next().now_or_never().is_some(),
            "response was buffered until EOF"
        );
    }
    assert!(notices.lock().unwrap().is_empty());
}

#[tokio::test]
async fn plain_json_starts_before_eof_and_restores_at_every_utf8_boundary() {
    use futures::FutureExt;
    let vault = vault("json-stream-progress");
    let secret = "synthetic-json-\"quote\"\\path\n🔑";
    vault.register(secret).unwrap();
    let mut request = json!({"content":secret});
    vault.protect(&mut request).unwrap();
    let wire = request.to_string();
    for size in [1, 2, 3, 7, 31, 1000] {
        let source = futures::stream::iter(
            wire.as_bytes()
                .chunks(size)
                .map(|chunk| Ok::<_, std::io::Error>(bytes::Bytes::copy_from_slice(chunk)))
                .collect::<Vec<_>>(),
        );
        let result = restore_stream(
            source,
            vault.clone(),
            StreamMode::Json,
            Arc::new(AtomicBool::new(false)),
        )
        .collect::<Vec<_>>()
        .await;
        let output = result
            .into_iter()
            .flat_map(|c| c.unwrap().to_vec())
            .collect::<Vec<_>>();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output).unwrap()["content"],
            secret
        );
    }
    let source = futures::stream::once(async {
        Ok::<_, std::io::Error>(bytes::Bytes::from_static(b"{\"content\":\"hello"))
    })
    .chain(futures::stream::pending());
    let mut output = restore_stream(
        source,
        vault,
        StreamMode::Json,
        Arc::new(AtomicBool::new(false)),
    );
    assert!(output.next().now_or_never().is_some());
}

#[tokio::test]
async fn json_transport_errors_release_held_original_bytes_once() {
    let wire = b"{\"content\":\"prefix <localrouter-secret id='0011";
    let source = futures::stream::iter(vec![
        Ok(bytes::Bytes::from_static(wire)),
        Err(std::io::Error::other("fixture disconnect")),
    ]);
    let output = restore_stream(
        source,
        vault("interrupted-json"),
        StreamMode::Json,
        Arc::new(AtomicBool::new(false)),
    )
    .collect::<Vec<_>>()
    .await;
    assert!(output.last().unwrap().is_err());
    let actual: Vec<u8> = output
        .into_iter()
        .filter_map(Result::ok)
        .flat_map(|c| c.to_vec())
        .collect();
    assert_eq!(actual, wire);
}

#[tokio::test]
async fn json_invalid_utf8_releases_held_original_bytes_once() {
    let first = b"{\"content\":\"prefix <localrouter-secret id='0011";
    let source = futures::stream::iter(vec![
        Ok::<_, std::io::Error>(bytes::Bytes::from_static(first)),
        Ok(bytes::Bytes::from_static(b"\xffsuffix")),
    ]);
    let output = restore_stream(
        source,
        vault("invalid-json-utf8"),
        StreamMode::Json,
        Arc::new(AtomicBool::new(false)),
    )
    .collect::<Vec<_>>()
    .await;
    let actual: Vec<u8> = output
        .into_iter()
        .flat_map(|c| c.unwrap().to_vec())
        .collect();
    assert_eq!(
        actual,
        [first.as_slice(), b"\xffsuffix".as_slice()].concat()
    );
}

#[tokio::test]
async fn json_unrelated_escapes_remain_byte_exact() {
    let wire = br#"{"text":"plain\n\u0061\ud83d\udd11 \/", "nested":{"value":"unchanged"}}"#;
    assert_eq!(
        restored(
            wire.chunks(1).map(|c| c.to_vec()).collect(),
            StreamMode::Json
        )
        .await,
        wire
    );
}

#[tokio::test]
async fn oversized_chunk_releases_preceding_json_carry_before_passthrough() {
    let first = b"{\"content\":\"prefix <localrouter-secret id='0011";
    let second = vec![b'x'; LIMIT + 1];
    let expected = [first.as_slice(), second.as_slice()].concat();
    let actual = restored(vec![first.to_vec(), second], StreamMode::Json).await;
    assert_eq!(actual, expected);
}

#[test]
fn multiline_request_values_are_masked_across_rolling_window_boundaries() {
    let vault = vault("rolling-request-value");
    let secret = "synthetic-rolling-original-374159";
    vault.register(secret).unwrap();
    let notices = warnings(&vault);
    let mut content = String::new();
    for i in 0..6000 {
        content.push_str(&format!("line {i}: {{\"html\":\"&amp;\",\"percent\":\"%41\",\"code\":\"hello\\nworld\"}} {secret}\n"));
    }
    let mut value = json!({"content":content});
    vault.protect(&mut value).unwrap();
    assert!(!value["content"].as_str().unwrap().contains(secret));
    assert!(notices.lock().unwrap().is_empty());
    let recovered: String = value["content"]
        .as_str()
        .unwrap()
        .split_inclusive('\n')
        .map(|line| vault.restore_text(line, false).unwrap())
        .collect();
    assert_eq!(recovered, content);
}

#[test]
fn rolling_request_cuts_do_not_split_owned_multiline_markers() {
    let vault = vault("rolling-owned-marker");
    vault.register("original-to-mask").unwrap();
    vault.register("Unmasked").unwrap();
    let mut seed = json!({"content":"original-to-mask"});
    vault.protect(&mut seed).unwrap();
    let tag = seed["content"]
        .as_str()
        .unwrap()
        .replace(" id=", "\nid=")
        .replace(" usage=", "\nusage=");
    let source = format!(
        "{}{tag}\n{}original-to-mask",
        "x\n".repeat(32750),
        "x\n".repeat(70000)
    );
    let mut request = json!({"content":source});
    vault.protect(&mut request).unwrap();
    let result = request["content"].as_str().unwrap();
    assert!(
        result.contains(&tag),
        "owned marker was changed by a window cut"
    );
    assert!(!result.contains("original-to-mask"));
}

#[tokio::test]
async fn long_whitespace_prefix_does_not_lock_in_an_incorrect_sse_hint() {
    use futures::FutureExt;
    let vault = vault("whitespace-sniff");
    let done = Arc::new(AtomicBool::new(false));
    let source = futures::stream::iter(vec![
        Ok::<_, std::io::Error>(bytes::Bytes::from(vec![b' '; 2048])),
        Ok(bytes::Bytes::from_static(b"{\"content\":\"complete\"}")),
    ])
    .chain(futures::stream::pending());
    let mut output = restore_stream(source, vault, StreamMode::Sse, done.clone());
    assert_eq!(output.next().await.unwrap().unwrap().len(), 2048);
    let second = output
        .next()
        .now_or_never()
        .expect("JSON was buffered until EOF")
        .unwrap()
        .unwrap();
    assert_eq!(second.as_ref(), b"{\"content\":\"complete\"}");
    assert!(done.load(Ordering::Acquire));
}

#[tokio::test]
async fn unrelated_code_is_byte_exact_through_fragmented_json_streams() {
    for content in [
        "const x = `value`;",
        "```js\nconst x = `hello`;\n```",
        "````json\n{\"x\":1}\n````",
        "can't haven't didn't <localrouter-secret",
        "&amp; %41 \\n",
    ] {
        let wire = json!({"content":content}).to_string().into_bytes();
        for size in [1, 2, 7] {
            let actual = restored(
                wire.chunks(size).map(|c| c.to_vec()).collect(),
                StreamMode::Json,
            )
            .await;
            assert_eq!(actual, wire);
        }
    }
}

#[tokio::test]
async fn transport_errors_preserve_a_staged_backtick_frame() {
    let first = format!(
        "data: {}\n\n",
        json!({"type":"response.output_text.delta","item_id":"text","delta":"`"})
    );
    let source = futures::stream::iter(vec![
        Ok(bytes::Bytes::from(first.clone())),
        Err(std::io::Error::other("fixture disconnect")),
    ]);
    let output = restore_stream(
        source,
        vault("staged-backtick"),
        StreamMode::Sse,
        Arc::new(AtomicBool::new(false)),
    )
    .collect::<Vec<_>>()
    .await;
    assert!(output.last().unwrap().is_err());
    let actual: Vec<u8> = output
        .into_iter()
        .filter_map(Result::ok)
        .flat_map(|c| c.to_vec())
        .collect();
    assert_eq!(actual, first.as_bytes());
}

#[test]
fn recovered_storage_preserves_old_and_outage_references_across_restart_and_purge() {
    struct Disk {
        inner: Arc<lr_api_keys::MockKeychain>,
        fail: AtomicBool,
    }
    impl KeychainStorage for Disk {
        fn get(&self, s: &str, a: &str) -> AppResult<Option<String>> {
            if self.fail.load(Ordering::SeqCst) {
                Err(AppError::Storage(
                    "No space left on device (os error 28)".into(),
                ))
            } else {
                self.inner.get(s, a)
            }
        }
        fn store(&self, s: &str, a: &str, v: &str) -> AppResult<()> {
            if self.fail.load(Ordering::SeqCst) {
                Err(AppError::Storage(
                    "No space left on device (os error 28)".into(),
                ))
            } else {
                self.inner.store(s, a, v)
            }
        }
        fn delete(&self, s: &str, a: &str) -> AppResult<()> {
            if self.fail.load(Ordering::SeqCst) {
                Err(AppError::Storage(
                    "No space left on device (os error 28)".into(),
                ))
            } else {
                self.inner.delete(s, a)
            }
        }
    }
    let durable = Arc::new(lr_api_keys::MockKeychain::new());
    let first = Arc::new(Vault::for_client(durable.clone(), "recovery-owner"));
    first.register("synthetic-durable-original").unwrap();
    let mut old = json!({"content":"synthetic-durable-original"});
    first.protect(&mut old).unwrap();
    let disk = Arc::new(Disk {
        inner: durable.clone(),
        fail: AtomicBool::new(true),
    });
    let storage = Arc::new(ResilientSecretStorage::new(disk.clone()));
    let vault = Arc::new(Vault::for_client(storage.clone(), "recovery-owner"));
    let notices = Arc::new(Mutex::new(Vec::new()));
    let sink = notices.clone();
    vault.set_operation_observer(Arc::new(move |notice| sink.lock().unwrap().push(notice)));
    vault.register("synthetic-outage-original").unwrap();
    let mut new = json!({"content":"synthetic-outage-original"});
    for _ in 0..10 {
        vault.protect(&mut new).unwrap();
    }
    assert_eq!(
        vault
            .restore_text(new["content"].as_str().unwrap(), true)
            .unwrap(),
        "synthetic-outage-original"
    );
    assert_eq!(
        notices
            .lock()
            .unwrap()
            .iter()
            .filter(|n| n.action == "storage_warning")
            .count(),
        1
    );
    assert!(notices
        .lock()
        .unwrap()
        .iter()
        .any(|n| n.location.contains("os error 28")));
    disk.fail.store(false, Ordering::SeqCst);
    storage.retry_persistence();
    assert!(!storage.persistence_warning());
    vault.protect(&mut json!({"content":"ordinary"})).unwrap();
    assert_eq!(
        notices
            .lock()
            .unwrap()
            .iter()
            .filter(|n| n.action == "storage_recovered")
            .count(),
        1
    );
    let fresh = Vault::find_client(
        durable.clone(),
        "recovery-owner",
        crate::placeholders::RETENTION_SECS,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        fresh
            .restore_text(old["content"].as_str().unwrap(), true)
            .unwrap(),
        "synthetic-durable-original"
    );
    assert_eq!(
        fresh
            .restore_text(new["content"].as_str().unwrap(), true)
            .unwrap(),
        "synthetic-outage-original"
    );
    let foreign = Vault::for_client(durable.clone(), "other-owner");
    assert_eq!(
        foreign
            .restore_text(new["content"].as_str().unwrap(), true)
            .unwrap(),
        new["content"].as_str().unwrap()
    );
    disk.fail.store(true, Ordering::SeqCst);
    Vault::purge_client(storage.clone(), "recovery-owner", None).unwrap();
    assert!(storage.persistence_warning());
    disk.fail.store(false, Ordering::SeqCst);
    storage.retry_persistence();
    assert!(!storage.persistence_warning());
    assert!(Vault::list_client(durable.clone(), "recovery-owner")
        .unwrap()
        .is_empty());
    let fresh = Vault::for_client(durable, "recovery-owner");
    assert_eq!(
        fresh
            .restore_text(old["content"].as_str().unwrap(), true)
            .unwrap(),
        old["content"].as_str().unwrap()
    );
    assert_eq!(
        fresh
            .restore_text(new["content"].as_str().unwrap(), true)
            .unwrap(),
        new["content"].as_str().unwrap()
    );
}

#[test]
fn search_json_restores_plaintext_without_rewriting_encrypted_output_or_protocol_ids() {
    let vault = vault("search-json-state");
    let secret = "synthetic\"quoted\\value\n🔎";
    vault.register(secret).unwrap();
    let mut request = json!({"id":secret,"model":"test","commands":{"search_query":[{"q":secret}]},"input":[{"type":"web_search_call","encrypted_content":secret}]});
    vault.protect(&mut request).unwrap();
    let tag = request["commands"]["search_query"][0]["q"]
        .as_str()
        .unwrap();
    assert_ne!(tag, secret);
    assert_eq!(request["id"], secret);
    assert_eq!(request["input"][0]["encrypted_content"], secret);
    let encrypted = format!("cipher:{tag}\\u0041");
    let response = json!({"encrypted_output":encrypted,"output":format!("Found {tag}"),"results":[{"type":"text_result","ref_id":"turn0search0","url":"https://example.com/","snippet":tag}],"future_field":{"flag":true}});
    let bytes = crate::wire::ResponseRestorer::new(vault.clone())
        .json(&serde_json::to_vec(&response).unwrap())
        .unwrap();
    let restored: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(restored["output"], format!("Found {secret}"));
    assert_eq!(restored["results"][0]["snippet"], secret);
    assert_eq!(restored["encrypted_output"], encrypted);
    assert_eq!(restored["results"][0]["ref_id"], "turn0search0");
    assert_eq!(restored["future_field"], response["future_field"]);
    let mut arguments = json!({"encrypted_output":tag});
    vault.restore(&mut arguments, true).unwrap();
    assert_eq!(
        arguments["encrypted_output"], secret,
        "application tool arguments are not provider state"
    );
}

#[tokio::test]
async fn fragmented_search_restoration_preserves_opaque_wire_and_client_isolation() {
    let vault = vault("search-fragment-owner");
    let secret = "synthetic\"quote\\path\n🔎";
    vault.register(secret).unwrap();
    let mut request = json!({"model":"test","commands":{"search_query":[{"q":secret}]}});
    vault.protect(&mut request).unwrap();
    let tag = request["commands"]["search_query"][0]["q"]
        .as_str()
        .unwrap();
    let opaque = format!("cipher:{tag}");
    let wire = json!({"encrypted_output":opaque,"output":tag,"results":[{"type":"text_result","ref_id":"turn0search0","snippet":tag}]}).to_string().replace("cipher:", "\\u0063ipher:");
    for chunk_size in [1, 3, 7, 29] {
        let source = futures::stream::iter(
            wire.as_bytes()
                .chunks(chunk_size)
                .map(|c| Ok::<_, std::io::Error>(bytes::Bytes::copy_from_slice(c)))
                .collect::<Vec<_>>(),
        );
        let output = restore_stream(
            source,
            vault.clone(),
            StreamMode::Json,
            Arc::new(AtomicBool::new(false)),
        )
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
        let output: Vec<u8> = output.into_iter().flat_map(|c| c.to_vec()).collect();
        let raw = String::from_utf8(output.clone()).unwrap();
        assert!(
            raw.contains("\\u0063ipher:"),
            "opaque JSON escapes must be forwarded verbatim"
        );
        let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(value["output"], secret);
        assert_eq!(value["results"][0]["snippet"], secret);
        assert_eq!(value["encrypted_output"], opaque);
    }
    let foreign = super::nonblocking_tests::vault("search-fragment-foreign");
    let source = futures::stream::iter(vec![Ok::<_, std::io::Error>(bytes::Bytes::from(
        wire.clone(),
    ))]);
    let chunks = restore_stream(
        source,
        foreign,
        StreamMode::Json,
        Arc::new(AtomicBool::new(false)),
    )
    .collect::<Vec<_>>()
    .await;
    let output: Vec<u8> = chunks
        .into_iter()
        .flat_map(|c| c.unwrap().to_vec())
        .collect();
    assert_eq!(output, wire.as_bytes());
}

#[tokio::test]
async fn search_json_starts_delivery_before_the_provider_finishes_its_body() {
    let vault = vault("search-rolling-delivery");
    let (send, receive) = tokio::sync::oneshot::channel::<()>();
    let source = async_stream::stream! {
        yield Ok::<_,std::io::Error>(bytes::Bytes::from_static(b"{\"encrypted_output\":\"opaque\",\"output\":\"Search prefix "));
        receive.await.unwrap();
        yield Ok(bytes::Bytes::from_static(b"tail\"}"));
    };
    let mut restored = restore_stream(
        source,
        vault,
        StreamMode::Json,
        Arc::new(AtomicBool::new(false)),
    );
    let first = tokio::time::timeout(std::time::Duration::from_secs(1), restored.next())
        .await
        .expect("the entire JSON response must not be buffered")
        .unwrap()
        .unwrap();
    assert!(!first.is_empty());
    send.send(()).unwrap();
    let mut body = first.to_vec();
    while let Some(chunk) = restored.next().await {
        body.extend_from_slice(&chunk.unwrap());
    }
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()["output"],
        "Search prefix tail"
    );
}
