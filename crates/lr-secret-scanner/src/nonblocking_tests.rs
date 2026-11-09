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
    assert!(!notices.lock().unwrap().is_empty());
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
