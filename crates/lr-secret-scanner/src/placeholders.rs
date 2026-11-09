//! Self-describing secret references and scoped native credential storage.
//! Originals use OS credentials in release builds and explicit development storage in debug builds.
use lr_api_keys::KeychainStorage;
use lr_types::{secret_context::SecretContext, AppError, AppResult};
use ring::{
    digest,
    rand::{SecureRandom, SystemRandom},
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use zeroize::Zeroizing;

pub const TAG: &str = "localrouter-redacted";
/// Original mappings expire after fourteen days without client activity.
pub const RETENTION_SECS: u64 = 14 * 24 * 60 * 60;
pub const MAX_TAG_BYTES: usize = 16 * 1024 * 1024; // The existing gateway request/response body budget.
const SERVICE: &str = "LocalRouter-Redacted-Secrets-v1";
const OWNERS: &str = "owner-index-v1";
static STORE_LOCK: Mutex<()> = Mutex::new(());
static ACTIVE: Mutex<BTreeMap<String, usize>> = Mutex::new(BTreeMap::new());
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn pin(account: &str) {
    *ACTIVE
        .lock()
        .expect("active secret scopes")
        .entry(account.into())
        .or_default() += 1;
}
fn unpin(account: &str) {
    let mut active = ACTIVE.lock().expect("active secret scopes");
    if let Some(n) = active.get_mut(account) {
        *n -= 1;
        if *n == 0 {
            active.remove(account);
        }
    }
}

fn error() -> AppError {
    AppError::InvalidParams(
        "Secret placeholder could not be resolved for this authenticated client".into(),
    )
}
fn parse_index(raw: &str) -> AppResult<Value> {
    let mut value: Value = serde_json::from_str(raw).map_err(|_| storage_error())?;
    let map = value.as_object_mut().ok_or_else(storage_error)?;
    for name in [
        "scopes",
        "responses",
        "revoked_scopes",
        "opaque",
        "revoked_ids",
    ] {
        let item = map.entry(name).or_insert_with(|| serde_json::json!({}));
        if !item.is_object() {
            return Err(storage_error());
        }
    }
    Ok(value)
}
fn storage_error() -> AppError {
    AppError::Internal(
        "OS credential storage for secret redaction is unavailable; request blocked".into(),
    )
}
fn key(owner: &str, scope: &str) -> String {
    let encoded = serde_json::to_vec(&(owner, scope)).expect("strings serialize");
    digest::digest(&digest::SHA256, &encoded)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn marker(id: &str) -> String {
    format!("<localrouter-redacted id='{id}' usage='Unmasked when returned verbatim'/>")
}

/// Scan a literal opening tag. This intentionally accepts XML-style markers,
/// not XML documents. The returned range never includes surrounding content.
fn candidate(text: &str, start: usize) -> Option<(usize, String)> {
    let bytes = text.as_bytes();
    let name_end = start.checked_add(1 + TAG.len())?;
    if bytes.get(start) != Some(&b'<') || !text.get(start + 1..name_end)?.eq_ignore_ascii_case(TAG)
    {
        return None;
    }
    if !bytes
        .get(name_end)
        .is_some_and(|b| b.is_ascii_whitespace() || matches!(b, b'/' | b'>'))
    {
        return None;
    }
    let limit = bytes.len().min(start.saturating_add(MAX_TAG_BYTES));
    let mut i = name_end;
    let mut id = None;
    loop {
        while i < limit && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= limit {
            return None;
        }
        if bytes[i] == b'/' {
            i += 1;
            if bytes.get(i) != Some(&b'>') {
                return None;
            }
        }
        if bytes.get(i) == Some(&b'>') {
            return Some((i + 1, id?));
        }
        let attr_start = i;
        while i < limit
            && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'-' | b'_' | b':'))
        {
            i += 1;
        }
        if attr_start == i {
            return None;
        }
        let is_id = text[attr_start..i].eq_ignore_ascii_case("id");
        while i < limit && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if bytes.get(i) != Some(&b'=') {
            return None;
        }
        i += 1;
        while i < limit && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let quote = *bytes.get(i)?;
        let value;
        if matches!(quote, b'\'' | b'"') {
            i += 1;
            let begin = i;
            while i < limit && bytes[i] != quote {
                if bytes[i] == b'<' {
                    return None;
                }
                i += 1;
            }
            if i >= limit {
                return None;
            }
            value = &text[begin..i];
            i += 1;
        } else {
            let begin = i;
            while i < limit && bytes[i].is_ascii_alphanumeric() {
                i += 1;
            }
            if begin == i {
                return None;
            }
            value = &text[begin..i];
        }
        if is_id {
            if id.is_some() || value.is_empty() || !value.bytes().all(|b| b.is_ascii_alphanumeric())
            {
                return None;
            }
            id = Some(value.to_ascii_lowercase());
        }
        if !bytes
            .get(i)
            .is_some_and(|b| b.is_ascii_whitespace() || matches!(b, b'/' | b'>'))
        {
            return None;
        }
    }
}

// JSON/code content may contain a tag whose attribute quotes are JSON escaped.
// Decode only a complete tag and JSON-escape only the inserted original.
fn reference(text: &str, start: usize) -> Option<(usize, String, bool)> {
    if let Some((end, id)) = candidate(text, start) {
        return Some((end, id, false));
    }
    let name_end = start.checked_add(1 + TAG.len())?;
    if !text.get(start + 1..name_end)?.eq_ignore_ascii_case(TAG) {
        return None;
    }
    let mut quote = None;
    for (offset, ch) in text.get(name_end..)?.char_indices() {
        if name_end + offset - start > MAX_TAG_BYTES {
            break;
        }
        match (quote, ch) {
            (Some(q), c) if q == c => quote = None,
            (None, '\'' | '"') => quote = Some(ch),
            (None, '>') => {
                let end = name_end + offset + 1;
                let decoded: String =
                    serde_json::from_str(&format!("\"{}\"", &text[start..end])).ok()?;
                let (decoded_end, id) = candidate(&decoded, 0)?;
                return (decoded_end == decoded.len()).then_some((end, id, true));
            }
            (_, '<') => return None,
            _ => {}
        }
    }
    None
}

/// Transform recognized references in one pass; do not recursively inspect inserted originals.
pub fn restore_text(
    text: &str,
    strict: bool,
    mut resolve: impl FnMut(&str) -> AppResult<Option<String>>,
) -> AppResult<String> {
    restore_text_depth(text, strict, &mut resolve, 0)
}

fn restore_text_depth(
    text: &str,
    strict: bool,
    resolve: &mut dyn FnMut(&str) -> AppResult<Option<String>>,
    depth: usize,
) -> AppResult<String> {
    if depth > 32 {
        return Err(AppError::InvalidParams(
            "Secret reference JSON nesting exceeds limit".into(),
        ));
    }
    // Decode only complete JSON values, one layer at a time. Serialize the
    // modified value again so inserted quotes, control characters and slashes
    // cannot change the surrounding JSON syntax.
    if text.to_ascii_lowercase().contains(TAG) || text.contains("\\u") {
        if let Ok(mut value) = serde_json::from_str::<Value>(text) {
            if value.is_object() || value.is_array() || value.is_string() {
                let original = value.clone();
                visit(
                    &mut value,
                    &mut |s, executable| {
                        restore_text_depth(s, strict || executable, resolve, depth + 1)
                    },
                    false,
                )?;
                if value != original {
                    return serde_json::to_string(&value).map_err(|_| error());
                }
                return Ok(text.to_owned());
            }
        }
    }
    let mut out = String::new();
    let mut offset = 0;
    for (start, _) in text.match_indices('<') {
        if start < offset {
            continue;
        }
        if let Some((end, id, escaped)) = reference(text, start) {
            out.push_str(&text[offset..start]);
            match resolve(&id)? {
                Some(value) => {
                    let encoded;
                    let value = if escaped {
                        encoded = serde_json::to_string(&value).map_err(|_| error())?;
                        &encoded[1..encoded.len() - 1]
                    } else {
                        value.as_str()
                    };
                    if out.len().saturating_add(value.len()) > MAX_TAG_BYTES {
                        return Err(AppError::InvalidParams(
                            "Restored content exceeds the gateway response budget".into(),
                        ));
                    }
                    out.push_str(value);
                }
                None if strict => return Err(error()),
                None => {
                    tracing::debug!("Unresolved secret reference left literal in non-executable response content");
                    out.push_str(&text[start..end]);
                }
            }
            offset = end;
        } else if strict
            && text.get(start + 1..).is_some_and(|s| {
                // Bare names in source strings are literal; use the same tag
                // boundary as candidate() before treating this as malformed.
                s.get(..TAG.len())
                    .is_some_and(|s| s.eq_ignore_ascii_case(TAG))
                    && s.as_bytes()
                        .get(TAG.len())
                        .is_some_and(|b| b.is_ascii_whitespace() || matches!(b, b'/' | b'>'))
            })
        {
            return Err(error());
        }
    }
    out.push_str(&text[offset..]);
    Ok(out)
}

/// Bounded streaming tag recognizer. UTF-8 and SSE frames are decoded by callers.
#[derive(Default)]
pub struct TextStream {
    pending: String,
    quote: Option<char>,
    json_depth: usize,
    json_string: bool,
    json_escape: bool,
    tag_in_json_string: bool,
}
impl TextStream {
    fn observe(&mut self, ch: char) {
        if self.json_string {
            if self.json_escape {
                self.json_escape = false;
            } else if ch == '\\' {
                self.json_escape = true;
            } else if ch == '"' {
                self.json_string = false;
            }
        } else if ch == '"' && self.json_depth > 0 {
            self.json_string = true;
        } else if matches!(ch, '{' | '[') {
            self.json_depth = self.json_depth.saturating_add(1);
        } else if matches!(ch, '}' | ']') {
            self.json_depth = self.json_depth.saturating_sub(1);
        }
    }

    pub(crate) fn buffered_bytes(&self) -> usize {
        self.pending.len()
    }
    pub fn push(
        &mut self,
        text: &str,
        finish: bool,
        mut restore: impl FnMut(&str) -> AppResult<String>,
    ) -> AppResult<String> {
        let mut ready = String::new();
        for ch in text.chars() {
            if self.pending.is_empty() {
                if ch == '<' {
                    self.tag_in_json_string = self.json_string;
                    self.pending.push(ch);
                } else {
                    self.observe(ch);
                    ready.push(ch);
                }
                continue;
            }
            let name_bytes = self.pending.len().saturating_sub(1);
            if name_bytes < TAG.len() {
                if ch.is_ascii() && ch.eq_ignore_ascii_case(&(TAG.as_bytes()[name_bytes] as char)) {
                    self.pending.push(ch);
                    continue;
                }
                ready.push_str(&std::mem::take(&mut self.pending));
                if ch == '<' {
                    self.tag_in_json_string = self.json_string;
                    self.pending.push(ch);
                } else {
                    self.observe(ch);
                    ready.push(ch);
                }
                continue;
            }
            if name_bytes == TAG.len() && !ch.is_ascii_whitespace() && ch != '/' && ch != '>' {
                ready.push_str(&std::mem::take(&mut self.pending));
                if ch == '<' {
                    self.tag_in_json_string = self.json_string;
                    self.pending.push(ch);
                } else {
                    self.observe(ch);
                    ready.push(ch);
                }
                continue;
            }
            if ch == '<' {
                // A nested opener cannot be part of a valid reference.
                ready.push_str(&std::mem::take(&mut self.pending));
                self.pending.push(ch);
                self.quote = None;
                continue;
            }
            self.pending.push(ch);
            match (self.quote, ch) {
                (Some(q), c) if q == c => self.quote = None,
                (None, '\'' | '"') => self.quote = Some(ch),
                (None, '>') => {
                    let mut restored = restore(&self.pending)?;
                    if self.tag_in_json_string
                        && restored != self.pending
                        && reference(&self.pending, 0).is_some_and(|(_, _, escaped)| !escaped)
                    {
                        let encoded = serde_json::to_string(&restored).map_err(|_| error())?;
                        restored = encoded[1..encoded.len() - 1].to_owned();
                    }
                    ready.push_str(&restored);
                    self.pending.clear();
                }
                _ => {}
            }
            if self.pending.len() >= MAX_TAG_BYTES {
                ready.push_str(&std::mem::take(&mut self.pending));
                self.quote = None;
            }
        }
        if finish {
            ready.push_str(&std::mem::take(&mut self.pending));
            self.quote = None;
        }
        // Complete tags above were already restored; do not rescan inserted originals.
        Ok(ready)
    }
}

// Opaque provider reasoning/compaction must stay byte-for-byte intact. A
// fingerprint authorizes only the references carried by that exact returned
// state, within the same authenticated owner and explicit conversation.
fn opaque_values(value: &Value) -> Vec<&str> {
    fn collect<'a>(value: &'a Value, out: &mut Vec<&'a str>) {
        match value {
            Value::Object(map) => {
                if map.get("type").and_then(Value::as_str).is_some_and(|kind| {
                    matches!(kind, "reasoning" | "compaction" | "compaction_summary")
                }) {
                    if let Some(text) = map
                        .get("encrypted_content")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                    {
                        out.push(text);
                    }
                }
                for value in map.values() {
                    collect(value, out);
                }
            }
            Value::Array(values) => {
                for value in values {
                    collect(value, out);
                }
            }
            _ => {}
        }
    }
    let mut values = Vec::new();
    collect(value, &mut values);
    values
}
/// A request capability over a durable owner/session record. Each request has
/// its own allowed-reference set; OS records are never searched globally.
type Detector = dyn Fn(&mut Value) -> AppResult<Vec<String>> + Send + Sync;
type AsyncDetector = dyn Fn(Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = AppResult<Value>> + Send>>
    + Send
    + Sync;
/// Safe store inventory; browsing does not refresh the owner's inactivity timer.
#[derive(Clone, Debug, serde::Serialize)]
pub struct StoredSecret {
    pub reference_id: String,
    pub placeholder: String,
    pub masked_value: String,
    pub last_used_at: u64,
    pub expires_at: u64,
    pub expired: bool,
    pub revoked: bool,
}

/// Metadata-only notification: original credential bytes never leave the vault.
#[derive(Clone, Debug)]
pub struct SecretOperation {
    pub action: &'static str,
    pub reference_id: String,
    pub location: String,
    pub occurrences: usize,
}
type OperationObserver = dyn Fn(SecretOperation) + Send + Sync;

fn reference_counts(text: &str) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    let mut offset = 0;
    for (start, _) in text.match_indices('<') {
        if start < offset {
            continue;
        }
        if let Some((end, id, _)) = reference(text, start) {
            *counts.entry(id).or_default() += 1;
            offset = end;
        }
    }
    counts
}

pub struct Vault {
    storage: Arc<dyn KeychainStorage>,
    account: String,
    owner: String,
    allowed: Mutex<HashSet<String>>,
    ephemeral: bool,
    retention_pin: bool,
    observer: Mutex<Option<Arc<OperationObserver>>>,
    epoch: Mutex<Option<u64>>,
    scanned: AtomicBool,
    restore_only: AtomicBool,
    detector: Mutex<Option<Arc<Detector>>>,
    async_detector: Mutex<Option<Arc<AsyncDetector>>>,
    contract: Mutex<crate::schema::ResponseContract>,
}
impl Vault {
    pub fn new(
        storage: Arc<dyn KeychainStorage>,
        owner: &str,
        session: &str,
        ephemeral: bool,
    ) -> Self {
        let account = key(owner, session);
        pin(&account);
        Self {
            storage,
            account,
            owner: owner.into(),
            allowed: Mutex::new(HashSet::new()),
            ephemeral,
            retention_pin: true,
            observer: Mutex::new(None),
            epoch: Mutex::new(None),
            scanned: AtomicBool::new(false),
            restore_only: AtomicBool::new(false),
            detector: Mutex::new(None),
            async_detector: Mutex::new(None),
            contract: Mutex::new(crate::schema::ResponseContract::default()),
        }
    }
    /// Continue resolving issued references without masking new outbound data.
    pub fn set_restore_only(&self) {
        self.restore_only.store(true, Ordering::Release);
    }
    pub fn set_detector(&self, detector: Arc<Detector>) {
        *self.detector.lock().expect("detector lock") = Some(detector);
    }
    pub fn set_request_contract(&self, request: &Value) -> AppResult<()> {
        // The provider sees masked schemas. Validate the eventual local values
        // against the original schema, including secret-valued enums and keys.
        let mut request = request.clone();
        fn restore_schemas(vault: &Vault, tools: &mut Value) -> AppResult<()> {
            if let Some(tools) = tools.as_array_mut() {
                for tool in tools {
                    if tool["type"] == "namespace" {
                        restore_schemas(vault, &mut tool["tools"])?;
                    } else {
                        let function = if tool.get("function").is_some() {
                            &mut tool["function"]
                        } else {
                            tool
                        };
                        for key in ["parameters", "input_schema"] {
                            if let Some(schema) = function.get_mut(key) {
                                visit(
                                    schema,
                                    &mut |text, _| {
                                        vault.restore_text_impl(
                                            text,
                                            false,
                                            "Contract validation",
                                            false,
                                        )
                                    },
                                    false,
                                )?;
                            }
                        }
                    }
                }
            }
            Ok(())
        }
        if let Some(tools) = request.get_mut("tools") {
            restore_schemas(self, tools)?;
        }
        if let Some(items) = request.get_mut("input").and_then(Value::as_array_mut) {
            for item in items
                .iter_mut()
                .filter(|item| item["type"] == "additional_tools")
            {
                restore_schemas(self, &mut item["tools"])?;
            }
        }
        for path in ["/response_format/json_schema/schema", "/text/format/schema"] {
            if let Some(schema) = request.pointer_mut(path) {
                visit(
                    schema,
                    &mut |text, _| {
                        self.restore_text_impl(text, false, "Contract validation", false)
                    },
                    false,
                )?;
            }
        }
        *self.contract.lock().map_err(|_| error())? =
            crate::schema::ResponseContract::from_request(
                &request,
                self.active() && !self.restore_only.load(Ordering::Acquire),
            )?;
        Ok(())
    }
    pub fn set_operation_observer(&self, observer: Arc<OperationObserver>) {
        *self.observer.lock().expect("secret operation observer") = Some(observer);
    }
    fn notify_operations(
        &self,
        action: &'static str,
        location: &str,
        counts: BTreeMap<String, usize>,
    ) {
        let observer = self
            .observer
            .lock()
            .ok()
            .and_then(|observer| observer.clone());
        if let Some(observer) = observer {
            for (reference_id, occurrences) in counts {
                observer(SecretOperation {
                    action,
                    reference_id,
                    location: location.into(),
                    occurrences,
                });
            }
        }
    }
    fn restore_text_impl(
        &self,
        text: &str,
        strict: bool,
        location: &str,
        notify: bool,
    ) -> AppResult<String> {
        if !text.to_ascii_lowercase().contains(TAG) && !text.contains("\\u") {
            return Ok(text.into());
        }
        let lock = STORE_LOCK.lock().map_err(|_| storage_error())?;
        let entries = self.load()?;
        let allowed = self.allowed.lock().map_err(|_| storage_error())?;
        let mut counts = BTreeMap::new();
        let restored = restore_text(
            text,
            strict && !self.restore_only.load(Ordering::Acquire),
            |id| {
                Ok(entries
                    .get(id)
                    .filter(|_| allowed.contains(id))
                    .map(|secret| {
                        *counts.entry(id.to_owned()).or_default() += 1;
                        secret.to_string()
                    }))
            },
        )?;
        drop(allowed);
        drop(entries);
        drop(lock);
        if notify {
            self.notify_operations("restore", location, counts);
        }
        Ok(restored)
    }
    pub fn set_async_detector(&self, detector: Arc<AsyncDetector>) {
        *self.async_detector.lock().expect("async secret detector") = Some(detector);
    }
    pub fn knows(&self, secret: &str) -> AppResult<bool> {
        let _lock = STORE_LOCK.lock().map_err(|_| storage_error())?;
        Ok(self.load()?.values().any(|s| s.as_str() == secret))
    }
    fn index_key(&self) -> String {
        format!("index-{}", key(&self.owner, ""))
    }
    fn index(&self) -> AppResult<Value> {
        match self
            .storage
            .get(SERVICE, &self.index_key())
            .map_err(|_| storage_error())?
        {
            Some(raw) => parse_index(&raw),
            None => Ok(serde_json::json!({"scopes":{},"responses":{},"revoked_scopes":{}})),
        }
    }
    fn save_index(&self, index: &Value) -> AppResult<()> {
        // Metadata only. Written first so removed clients and interrupted native
        // deletions remain discoverable across application restarts.
        let mut owners = Self::registered_owners(self.storage.as_ref())?;
        if owners.insert(self.owner.clone()) {
            self.storage
                .store(
                    SERVICE,
                    OWNERS,
                    &serde_json::to_string(&owners).map_err(|_| storage_error())?,
                )
                .map_err(|_| storage_error())?;
        }
        self.storage
            .store(SERVICE, &self.index_key(), &index.to_string())
            .map_err(|_| storage_error())
    }
    pub fn bind_opaque(&self, value: &Value) -> AppResult<()> {
        let values = opaque_values(value);
        if values.is_empty() || !self.active() {
            return Ok(());
        }
        let _lock = STORE_LOCK.lock().map_err(|_| storage_error())?;
        let entries = self.load()?;
        let ids = self
            .allowed
            .lock()
            .map_err(|_| storage_error())?
            .iter()
            .filter(|id| entries.contains_key(*id))
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        let mut index = self.index()?;
        if index.get("opaque").is_none() {
            index["opaque"] = serde_json::json!({});
        }
        for value in values {
            index["opaque"][key("opaque", value)] =
                serde_json::json!({"account":self.account,"ids":ids});
        }
        index["scopes"][&self.account] = Value::from(now());
        self.save_index(&index)
    }
    pub fn bind_response(&self, id: &str) -> AppResult<()> {
        let _lock = STORE_LOCK.lock().map_err(|_| storage_error())?;
        let entries = self.load()?;
        self.save(&entries)?;
        let mut index = self.index()?;
        index["responses"][id] = Value::String(self.account.clone());
        self.save_index(&index)
    }
    /// Resolve an owned provider response binding. Provider IDs are opaque;
    /// an untracked ID is not an invalid request and grants no secret access.
    /// Storage failures and explicit revocations still fail closed.
    pub fn find_response(
        storage: Arc<dyn KeychainStorage>,
        owner: &str,
        id: &str,
    ) -> AppResult<Option<Self>> {
        let mut vault = Self::new(storage, owner, "", false);
        let _lock = STORE_LOCK.lock().map_err(|_| storage_error())?;
        let index = vault.index()?;
        if index["revoked"] == true {
            return Err(error());
        }
        let Some(binding) = index["responses"].get(id) else {
            return Ok(None);
        };
        let account = binding.as_str().ok_or_else(storage_error)?.to_owned();
        if index["revoked_scopes"][&account] == true {
            return Err(error());
        }
        unpin(&vault.account);
        vault.account = account;
        pin(&vault.account);
        if vault
            .storage
            .get(SERVICE, &vault.account)
            .map_err(|_| storage_error())?
            .is_none()
        {
            return Ok(None);
        }
        let entries = vault.load()?;
        vault
            .allowed
            .lock()
            .map_err(|_| storage_error())?
            .extend(entries.keys().cloned());
        Ok(Some(vault))
    }
    /// An explicit authenticated owner/session identity can recover only its
    /// own native record, including when the provider omits continuation IDs.
    pub fn find_session(
        storage: Arc<dyn KeychainStorage>,
        owner: &str,
        session: &str,
    ) -> AppResult<Option<Self>> {
        let vault = Self::new(storage, owner, session, false);
        let _lock = STORE_LOCK.lock().map_err(|_| storage_error())?;
        let index = vault.index()?;
        if index["revoked"] == true || index["revoked_scopes"][&vault.account] == true {
            return Err(error());
        }
        if vault
            .storage
            .get(SERVICE, &vault.account)
            .map_err(|_| storage_error())?
            .is_none()
        {
            return Ok(None);
        }
        let entries = vault.load()?;
        vault
            .allowed
            .lock()
            .map_err(|_| storage_error())?
            .extend(entries.keys().cloned());
        Ok(Some(vault))
    }
    pub fn find_scope(
        storage: Arc<dyn KeychainStorage>,
        owner: &str,
        previous: Option<&str>,
        session: Option<&str>,
    ) -> AppResult<Option<Self>> {
        if let Some(id) = previous {
            if let Some(vault) = Self::find_response(storage.clone(), owner, id)? {
                return Ok(Some(vault));
            }
        }
        match session {
            Some(id) => Self::find_session(storage, owner, id),
            None => Ok(None),
        }
    }
    /// Persistent mappings belong to an authenticated client. Session/response
    /// identities are diagnostic links, never required to recover its secrets.
    /// Preserve existing IDs when upgrading conversation-based records.
    pub fn find_client(
        storage: Arc<dyn KeychainStorage>,
        owner: &str,
        retention_secs: u64,
    ) -> AppResult<Option<Self>> {
        let vault = Self::new(storage.clone(), owner, "client-secret-map-v2", false);
        let _lock = STORE_LOCK.lock().map_err(|_| storage_error())?;
        let mut index = vault.index()?;
        if index["revoked"] == true {
            return Err(error());
        }
        if index["scopes"][&vault.account]
            .as_u64()
            .is_some_and(|updated| now().saturating_sub(updated) >= retention_secs)
            && ACTIVE
                .lock()
                .map_err(|_| storage_error())?
                .get(&vault.account)
                .copied()
                .unwrap_or_default()
                <= 1
        {
            index["revoked_scopes"][&vault.account] = Value::Bool(true);
            vault.save_index(&index)?;
            vault
                .storage
                .delete(SERVICE, &vault.account)
                .map_err(|_| storage_error())?;
            index["scopes"]
                .as_object_mut()
                .ok_or_else(storage_error)?
                .remove(&vault.account);
            index["responses"]
                .as_object_mut()
                .ok_or_else(storage_error)?
                .retain(|_, v| v.as_str() != Some(&vault.account));
            index["opaque"]
                .as_object_mut()
                .ok_or_else(storage_error)?
                .retain(|_, v| v["account"].as_str() != Some(&vault.account));
            index["revoked_scopes"]
                .as_object_mut()
                .ok_or_else(storage_error)?
                .remove(&vault.account);
            vault.save_index(&index)?;
            return Ok(None);
        }
        let mut entries = vault.load()?;
        if index["client_record"].as_str() != Some(&vault.account) {
            let scopes = index["scopes"]
                .as_object()
                .ok_or_else(storage_error)?
                .clone();
            let mut migrated = HashSet::new();
            for (account, updated) in scopes {
                if account == vault.account
                    || index["revoked_scopes"][&account] == true
                    || now().saturating_sub(updated.as_u64().ok_or_else(storage_error)?)
                        >= retention_secs
                {
                    continue;
                }
                let mut legacy = Self::new(storage.clone(), owner, "", false);
                unpin(&legacy.account);
                legacy.account = account.clone();
                pin(&legacy.account);
                for (id, value) in legacy.load()? {
                    if entries.get(&id).is_some_and(|existing| existing != &value) {
                        return Err(storage_error());
                    }
                    entries.insert(id, value);
                }
                migrated.insert(account);
            }
            if entries.is_empty() {
                return Ok(None);
            }
            vault.save(&entries)?;
            index = vault.index()?;
            if let Some(responses) = index["responses"].as_object_mut() {
                for account in responses.values_mut() {
                    if account.as_str().is_some_and(|a| migrated.contains(a)) {
                        *account = Value::String(vault.account.clone());
                    }
                }
            }
            if let Some(opaque) = index["opaque"].as_object_mut() {
                for binding in opaque.values_mut() {
                    if binding["account"]
                        .as_str()
                        .is_some_and(|a| migrated.contains(a))
                    {
                        binding["account"] = Value::String(vault.account.clone());
                    }
                }
            }
            index["client_record"] = Value::String(vault.account.clone());
            vault.save_index(&index)?;
        }
        if entries.is_empty() {
            return Ok(None);
        }
        index["scopes"][&vault.account] = Value::from(now());
        vault.save_index(&index)?;
        vault
            .allowed
            .lock()
            .map_err(|_| storage_error())?
            .extend(entries.keys().cloned());
        Ok(Some(vault))
    }

    pub fn client_scoped(&self) -> bool {
        self.account == key(&self.owner, "client-secret-map-v2")
    }

    pub fn for_client(storage: Arc<dyn KeychainStorage>, owner: &str) -> Self {
        Self::new(storage, owner, "client-secret-map-v2", false)
    }

    pub fn continue_response(
        storage: Arc<dyn KeychainStorage>,
        owner: &str,
        id: &str,
    ) -> AppResult<Self> {
        Self::find_response(storage, owner, id)?.ok_or_else(error)
    }
    fn registered_owners(
        storage: &dyn KeychainStorage,
    ) -> AppResult<std::collections::BTreeSet<String>> {
        storage
            .get(SERVICE, OWNERS)
            .map_err(|_| storage_error())?
            .map(|raw| serde_json::from_str(&raw).map_err(|_| storage_error()))
            .transpose()
            .map(Option::unwrap_or_default)
    }
    pub fn stored_owners(
        storage: &dyn KeychainStorage,
    ) -> AppResult<std::collections::BTreeSet<String>> {
        let _lock = STORE_LOCK.lock().map_err(|_| storage_error())?;
        Self::registered_owners(storage)
    }
    /// Retry cleanup for owners removed from settings, including after restart.
    pub fn cleanup_registered(
        storage: Arc<dyn KeychainStorage>,
        live_owners: &[String],
        ttl_secs: u64,
    ) -> AppResult<()> {
        let mut owners = {
            let _lock = STORE_LOCK.lock().map_err(|_| storage_error())?;
            Self::registered_owners(storage.as_ref())?
        };
        // Adopt live scopes created before the durable owner registry existed.
        owners.extend(live_owners.iter().cloned());
        let mut failed = false;
        for owner in owners {
            let result = if live_owners.contains(&owner) {
                Self::cleanup_client(storage.clone(), &owner, ttl_secs)
            } else {
                Self::forget_client(storage.clone(), &owner)
            };
            if result.is_err() {
                failed = true;
            }
        }
        if failed {
            Err(storage_error())
        } else {
            Ok(())
        }
    }
    pub fn cleanup_client(
        storage: Arc<dyn KeychainStorage>,
        owner: &str,
        ttl_secs: u64,
    ) -> AppResult<()> {
        let vault = Self::new(storage, owner, "\0cleanup", false);
        let _lock = STORE_LOCK.lock().map_err(|_| storage_error())?;
        let Some(raw) = vault
            .storage
            .get(SERVICE, &vault.index_key())
            .map_err(|_| storage_error())?
        else {
            return Ok(());
        };
        let mut index = parse_index(&raw)?;
        let scopes = index["scopes"].as_object().ok_or_else(storage_error)?;
        let expired: Vec<_> = scopes
            .iter()
            .filter(|(account, updated)| {
                now().saturating_sub(updated.as_u64().unwrap_or_default()) >= ttl_secs
                    && !ACTIVE
                        .lock()
                        .expect("active secret scopes")
                        .contains_key(*account)
            })
            .map(|(account, _)| account.clone())
            .collect();
        for account in &expired {
            index["revoked_scopes"][account] = Value::Bool(true);
            // Revoke references before attempting native deletion. Failed deletion
            // leaves the account indexed for a future cleanup attempt.
            if let Some(responses) = index["responses"].as_object_mut() {
                responses.retain(|_, v| v.as_str() != Some(account));
            }
            if let Some(opaque) = index.get_mut("opaque").and_then(Value::as_object_mut) {
                opaque.retain(|_, value| value["account"].as_str() != Some(account));
            }
        }
        vault.save_index(&index)?;
        for account in expired {
            vault
                .storage
                .delete(SERVICE, &account)
                .map_err(|_| storage_error())?;
            index["scopes"]
                .as_object_mut()
                .expect("scope index")
                .remove(&account);
            if let Some(revoked) = index["revoked_scopes"].as_object_mut() {
                revoked.remove(&account);
            }
        }
        vault.save_index(&index)
    }
    pub fn list_client(
        storage: Arc<dyn KeychainStorage>,
        owner: &str,
    ) -> AppResult<Vec<StoredSecret>> {
        let vault = Self::for_client(storage, owner);
        let _lock = STORE_LOCK.lock().map_err(|_| storage_error())?;
        let index = vault.index()?;
        let mut values: BTreeMap<String, (Zeroizing<String>, u64, bool)> = BTreeMap::new();
        for (account, updated) in index["scopes"].as_object().ok_or_else(storage_error)? {
            let updated = updated.as_u64().ok_or_else(storage_error)?;
            for (id, original) in vault.load_record(account)? {
                let revoked = index["revoked"] == true
                    || index["revoked_scopes"][account] == true
                    || index["revoked_ids"][&id] == true;
                if let Some((existing, timestamp, existing_revoked)) = values.get_mut(&id) {
                    if existing.as_str() != original.as_str() {
                        return Err(storage_error());
                    }
                    *timestamp = (*timestamp).max(updated);
                    *existing_revoked |= revoked;
                } else {
                    values.insert(id, (original, updated, revoked));
                }
            }
        }
        Ok(values
            .into_iter()
            .map(|(reference_id, (original, last_used_at, revoked))| {
                let chars: Vec<_> = original.chars().collect();
                let masked_value = if chars.len() <= 16 {
                    "********".into()
                } else {
                    format!(
                        "{}…{}",
                        chars[..6].iter().collect::<String>(),
                        chars[chars.len() - 4..].iter().collect::<String>()
                    )
                };
                let expires_at = last_used_at.saturating_add(RETENTION_SECS);
                StoredSecret {
                    placeholder: marker(&reference_id),
                    reference_id,
                    masked_value,
                    last_used_at,
                    expires_at,
                    expired: now() >= expires_at,
                    revoked,
                }
            })
            .collect())
    }

    /// Purge one original and all its aliases, or the whole client map. Revocation
    /// precedes native writes/deletes; an interrupted purge cannot restore it.
    pub fn purge_client(
        storage: Arc<dyn KeychainStorage>,
        owner: &str,
        reference_id: Option<&str>,
    ) -> AppResult<usize> {
        let vault = Self::for_client(storage, owner);
        let _lock = STORE_LOCK.lock().map_err(|_| storage_error())?;
        let mut index = vault.index()?;
        let accounts: Vec<_> = index["scopes"]
            .as_object()
            .ok_or_else(storage_error)?
            .keys()
            .cloned()
            .collect();
        let mut records = BTreeMap::new();
        for account in accounts {
            records.insert(account.clone(), vault.load_record(&account)?);
        }
        let target = if let Some(id) = reference_id {
            let id = id.to_ascii_lowercase();
            let Some(target) = records
                .values()
                .find_map(|entries| entries.get(&id))
                .cloned()
            else {
                return Ok(0);
            };
            Some(target)
        } else {
            None
        };
        let mut removed = HashSet::new();
        for entries in records.values_mut() {
            entries.retain(|id, secret| {
                if target
                    .as_ref()
                    .is_none_or(|target| target.as_str() == secret.as_str())
                {
                    removed.insert(id.clone());
                    false
                } else {
                    true
                }
            });
        }
        if records.is_empty() {
            return Ok(0);
        }
        let epoch = index["epoch"]
            .as_u64()
            .unwrap_or_default()
            .checked_add(1)
            .ok_or_else(storage_error)?;
        index["epoch"] = epoch.into();
        if reference_id.is_none() {
            index["revoked"] = true.into();
        }
        for id in &removed {
            index["revoked_ids"][id] = true.into();
        }
        vault.save_index(&index)?;
        for (account, entries) in records {
            if entries.is_empty() {
                vault
                    .storage
                    .delete(SERVICE, &account)
                    .map_err(|_| storage_error())?;
                index["scopes"]
                    .as_object_mut()
                    .ok_or_else(storage_error)?
                    .remove(&account);
                index["responses"]
                    .as_object_mut()
                    .ok_or_else(storage_error)?
                    .retain(|_, value| value.as_str() != Some(&account));
                index["opaque"]
                    .as_object_mut()
                    .ok_or_else(storage_error)?
                    .retain(|_, value| value["account"].as_str() != Some(&account));
                index["revoked_scopes"]
                    .as_object_mut()
                    .ok_or_else(storage_error)?
                    .remove(&account);
            } else {
                let entries: serde_json::Map<_, _> = entries
                    .iter()
                    .map(|(id, secret)| (id.clone(), Value::String(secret.to_string())))
                    .collect();
                let record = Zeroizing::new(
                    serde_json::json!({"owner":owner,"entries":entries}).to_string(),
                );
                vault
                    .storage
                    .store(SERVICE, &account, &record)
                    .map_err(|_| storage_error())?;
            }
        }
        if reference_id.is_none() {
            index["revoked"] = false.into();
            index["revoked_ids"] = serde_json::json!({});
            index["client_record"] = vault.account.clone().into();
        }
        vault.save_index(&index)?;
        Ok(removed.len())
    }

    pub fn forget_client(storage: Arc<dyn KeychainStorage>, owner: &str) -> AppResult<()> {
        let vault = Self::new(storage, owner, "", false);
        let _lock = STORE_LOCK.lock().map_err(|_| storage_error())?;
        let Some(raw) = vault
            .storage
            .get(SERVICE, &vault.index_key())
            .map_err(|_| storage_error())?
        else {
            return Ok(());
        };
        let mut index = parse_index(&raw)?;
        let accounts: Vec<String> = index["scopes"]
            .as_object()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        index["revoked"] = Value::Bool(true);
        vault.save_index(&index)?;
        for account in accounts {
            vault
                .storage
                .delete(SERVICE, &account)
                .map_err(|_| storage_error())?;
        }
        // Retain the revocation tombstone: an already-running request cannot re-create this client's secrets.
        index["scopes"] = serde_json::json!({});
        index["responses"] = serde_json::json!({});
        index["opaque"] = serde_json::json!({});
        vault.save_index(&index)
    }
    fn check_epoch(&self, index: &Value) -> AppResult<()> {
        let current = index["epoch"].as_u64().unwrap_or_default();
        let mut epoch = self.epoch.lock().map_err(|_| storage_error())?;
        if epoch.is_some_and(|epoch| epoch != current) {
            return Err(error());
        }
        *epoch = Some(current);
        Ok(())
    }
    fn load(&self) -> AppResult<BTreeMap<String, Zeroizing<String>>> {
        let index = self.index()?;
        if index["revoked"] == true || index["revoked_scopes"][&self.account] == true {
            return Err(error());
        }
        self.check_epoch(&index)?;
        let mut entries = self.load_record(&self.account)?;
        entries.retain(|id, _| index["revoked_ids"][id] != true);
        Ok(entries)
    }
    fn load_record(&self, account: &str) -> AppResult<BTreeMap<String, Zeroizing<String>>> {
        let Some(raw) = self
            .storage
            .get(SERVICE, account)
            .map_err(|_| storage_error())?
        else {
            return Ok(BTreeMap::new());
        };
        let raw = Zeroizing::new(raw);
        let data: Value = serde_json::from_str(&raw).map_err(|_| storage_error())?;
        if data["owner"].as_str() != Some(&self.owner) {
            return Err(storage_error());
        }
        let entries = data["entries"].as_object().ok_or_else(storage_error)?;
        entries
            .iter()
            .map(|(id, value)| {
                Ok((
                    id.clone(),
                    Zeroizing::new(value.as_str().ok_or_else(storage_error)?.to_owned()),
                ))
            })
            .collect()
    }
    fn save(&self, entries: &BTreeMap<String, Zeroizing<String>>) -> AppResult<()> {
        let values: serde_json::Map<String, Value> = entries
            .iter()
            .map(|(id, s)| (id.clone(), Value::String(s.to_string())))
            .collect();
        let data = Zeroizing::new(
            serde_json::to_string(&serde_json::json!({"owner":self.owner,"entries":values}))
                .map_err(|_| storage_error())?,
        );
        let mut index = self.index()?;
        if index["revoked"] == true {
            return Err(error());
        }
        self.check_epoch(&index)?;
        index["scopes"][&self.account] = Value::from(now());
        // Index first: a failed value write remains discoverable for cleanup.
        self.save_index(&index)?;
        self.storage
            .store(SERVICE, &self.account, &data)
            .map_err(|_| storage_error())
    }
    pub fn forget(&self) -> AppResult<()> {
        let _lock = STORE_LOCK.lock().map_err(|_| storage_error())?;
        self.allowed.lock().map_err(|_| storage_error())?.clear();
        let mut index = self.index()?;
        index["revoked_scopes"][&self.account] = Value::Bool(true);
        if let Some(responses) = index["responses"].as_object_mut() {
            responses.retain(|_, v| v.as_str() != Some(&self.account));
        }
        if let Some(opaque) = index.get_mut("opaque").and_then(Value::as_object_mut) {
            opaque.retain(|_, value| value["account"].as_str() != Some(&self.account));
        }
        self.save_index(&index)?;
        self.storage
            .delete(SERVICE, &self.account)
            .map_err(|_| storage_error())?;
        if let Some(scopes) = index["scopes"].as_object_mut() {
            scopes.remove(&self.account);
        }
        if let Some(revoked) = index["revoked_scopes"].as_object_mut() {
            revoked.remove(&self.account);
        }
        self.save_index(&index)
    }
    fn transform(
        &self,
        text: &str,
        entries: &mut BTreeMap<String, Zeroizing<String>>,
    ) -> AppResult<String> {
        let preserved: Vec<_> = text
            .match_indices('<')
            .filter_map(|(start, _)| {
                candidate(text, start)
                    .filter(|(_, id)| entries.contains_key(id))
                    .map(|(end, _)| (start, end))
            })
            .collect();
        let mut spans = Vec::new();
        for value in entries.values() {
            let mut offset = 0;
            while let Some(found) = text[offset..].find(value.as_str()) {
                let start = offset + found;
                let end = start + value.len();
                if !preserved.iter().any(|(a, b)| start < *b && end > *a) {
                    spans.push((start, end));
                }
                offset = start
                    + text[start..]
                        .chars()
                        .next()
                        .expect("nonempty match")
                        .len_utf8();
            }
        }
        spans.sort_unstable();
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for (a, b) in spans {
            if let Some(last) = merged.last_mut().filter(|last| a < last.1) {
                last.1 = last.1.max(b);
            } else {
                merged.push((a, b));
            }
        }
        let mut out = String::new();
        let mut offset = 0;
        for (a, b) in merged {
            let original = &text[a..b];
            let id = match entries.iter().find(|(_, s)| s.as_str() == original) {
                Some((id, _)) => id.clone(),
                None => Self::insert(entries, original)?,
            };
            self.allowed
                .lock()
                .map_err(|_| storage_error())?
                .insert(id.clone());
            out.push_str(&text[offset..a]);
            out.push_str(&marker(&id));
            offset = b;
        }
        out.push_str(&text[offset..]);
        Ok(out)
    }
    fn insert(
        entries: &mut BTreeMap<String, Zeroizing<String>>,
        secret: &str,
    ) -> AppResult<String> {
        loop {
            let mut bytes = [0u8; 16];
            SystemRandom::new()
                .fill(&mut bytes)
                .map_err(|_| storage_error())?;
            let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            if !entries.contains_key(&id) {
                entries.insert(id.clone(), Zeroizing::new(secret.to_owned()));
                return Ok(id);
            }
        }
    }
}
impl Drop for Vault {
    fn drop(&mut self) {
        if self.retention_pin {
            unpin(&self.account);
        }
        if self.ephemeral {
            if self.forget().is_err() {
                tracing::warn!("Could not clean up request-local secret credential record");
            }
        }
    }
}
impl SecretContext for Vault {
    fn monitor_context(&self) -> Option<Arc<dyn SecretContext>> {
        let mut context = Self::new(self.storage.clone(), &self.owner, "", false);
        unpin(&context.account);
        context.account = self.account.clone();
        context.retention_pin = false;
        let epoch = *self.epoch.lock().expect("secret owner epoch");
        *context.epoch.lock().expect("diagnostic owner epoch") = epoch;
        if self.restore_only.load(Ordering::Acquire) {
            context.set_restore_only();
        }
        Some(Arc::new(context))
    }
    fn redact_monitor(&self, value: &mut Value) -> AppResult<()> {
        if self.restore_only.load(Ordering::Acquire) {
            return Ok(());
        }
        let _lock = STORE_LOCK.lock().map_err(|_| storage_error())?;
        let entries = self.load()?;
        let mut replacements = Vec::new();
        for (id, secret) in &entries {
            let tag = marker(id);
            let encoded = serde_json::to_string(secret.as_str()).map_err(|_| storage_error())?;
            let escaped = &encoded[1..encoded.len() - 1];
            if escaped != secret.as_str() {
                let encoded_tag = serde_json::to_string(&tag).map_err(|_| storage_error())?;
                replacements.push((
                    escaped.to_owned(),
                    encoded_tag[1..encoded_tag.len() - 1].to_owned(),
                ));
            }
            replacements.push((secret.to_string(), tag));
        }
        replacements.sort_by_key(|(secret, _)| std::cmp::Reverse(secret.len()));
        let ids: HashSet<_> = entries.keys().cloned().collect();
        fn mask(text: &str, replacements: &[(String, String)], ids: &HashSet<String>) -> String {
            let preserved: Vec<_> = text
                .match_indices('<')
                .filter_map(|(start, _)| {
                    reference(text, start)
                        .filter(|(_, id, _)| ids.contains(id))
                        .map(|(end, _, _)| (start, end))
                })
                .collect();
            let mut masked = String::new();
            let mut offset = 0;
            loop {
                let next = replacements
                    .iter()
                    .filter_map(|(secret, tag)| {
                        text[offset..]
                            .match_indices(secret)
                            .map(|(found, _)| offset + found)
                            .find(|start| {
                                !preserved
                                    .iter()
                                    .any(|(a, b)| *start < *b && *start + secret.len() > *a)
                            })
                            .map(|start| (start, secret, tag))
                    })
                    .min_by_key(|(start, secret, _)| (*start, std::cmp::Reverse(secret.len())));
                let Some((start, secret, tag)) = next else {
                    break;
                };
                masked.push_str(&text[offset..start]);
                masked.push_str(tag);
                offset = start + secret.len();
            }
            masked.push_str(&text[offset..]);
            masked
        }
        fn walk(
            value: &mut Value,
            replacements: &[(String, String)],
            ids: &HashSet<String>,
            root: bool,
        ) {
            match value {
                Value::String(text) => *text = mask(text, replacements, ids),
                Value::Array(items) => items
                    .iter_mut()
                    .for_each(|v| walk(v, replacements, ids, false)),
                Value::Object(map) => {
                    let entries = std::mem::take(map);
                    for (key, mut value) in entries {
                        walk(&mut value, replacements, ids, false);
                        map.insert(
                            if root {
                                key
                            } else {
                                mask(&key, replacements, ids)
                            },
                            value,
                        );
                    }
                }
                _ => {}
            }
        }
        walk(value, &replacements, &ids, true);
        Ok(())
    }
    fn register(&self, secret: &str) -> AppResult<()> {
        if secret.is_empty() {
            return Ok(());
        }
        let _lock = STORE_LOCK.lock().map_err(|_| storage_error())?;
        let mut entries = self.load()?;
        if !entries.values().any(|s| s.as_str() == secret) {
            Self::insert(&mut entries, secret)?;
            self.save(&entries)?;
        }
        Ok(())
    }
    fn prepare<'a>(
        &'a self,
        value: &'a mut Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = AppResult<()>> + Send + 'a>> {
        Box::pin(async move {
            // Mask known originals first; only newly introduced secrets need a decision.
            self.protect(value)?;
            let detector = self
                .async_detector
                .lock()
                .map_err(|_| storage_error())?
                .clone();
            if let Some(detector) = detector {
                *value = detector(value.clone()).await?;
            }
            self.protect(value)?;
            if self.active() {
                self.contract
                    .lock()
                    .map_err(|_| error())?
                    .check_placeholders()?;
            }
            Ok(())
        })
    }
    fn protect(&self, value: &mut Value) -> AppResult<()> {
        if self.restore_only.load(Ordering::Acquire) {
            return Ok(());
        }

        let detector = self.detector.lock().map_err(|_| storage_error())?.clone();
        if let Some(detector) = detector {
            for secret in detector(value)? {
                self.register(&secret)?;
            }
        }
        let _lock = STORE_LOCK.lock().map_err(|_| storage_error())?;
        let mut entries = self.load()?;
        let index = self.index()?;
        for opaque in opaque_values(value) {
            let binding = &index["opaque"][key("opaque", opaque)];
            if binding["account"].as_str() == Some(&self.account) {
                if let Some(ids) = binding["ids"].as_array() {
                    self.allowed.lock().map_err(|_| storage_error())?.extend(
                        ids.iter()
                            .filter_map(Value::as_str)
                            .filter(|id| entries.contains_key(*id))
                            .map(str::to_owned),
                    );
                }
            }
        }
        let count = entries.len();
        let mut redacted = BTreeMap::new();
        fn signed_secret(value: &Value, entries: &BTreeMap<String, Zeroizing<String>>) -> bool {
            fn contains(value: &Value, entries: &BTreeMap<String, Zeroizing<String>>) -> bool {
                match value {
                    Value::String(text) => entries
                        .values()
                        .any(|secret| text.contains(secret.as_str())),
                    Value::Object(map) => map.values().any(|v| contains(v, entries)),
                    Value::Array(values) => values.iter().any(|v| contains(v, entries)),
                    _ => false,
                }
            }
            match value {
                Value::Object(map) => {
                    if ["signature", "thought_signature"].iter().any(|key| {
                        map.get(*key)
                            .and_then(Value::as_str)
                            .is_some_and(|s| !s.is_empty())
                    }) {
                        contains(value, entries)
                    } else {
                        map.get("encrypted_content")
                            .is_some_and(|v| contains(v, entries))
                            || map.values().any(|v| signed_secret(v, entries))
                    }
                }
                Value::Array(values) => values.iter().any(|v| signed_secret(v, entries)),
                _ => false,
            }
        }
        if signed_secret(value, &entries) {
            return Err(AppError::InvalidParams(
                "A signed provider block contains a secret and cannot be safely redacted".into(),
            ));
        }
        visit_with_mode(
            value,
            &mut |text, _| {
                // Existing references in the authenticated session history are capabilities
                // for this request, too. Never authorize references from another record.
                restore_text(text, false, |id| {
                    if entries.contains_key(id) {
                        self.allowed
                            .lock()
                            .map_err(|_| storage_error())?
                            .insert(id.into());
                    }
                    Ok(None)
                })?;
                let masked = self.transform(text, &mut entries)?;
                if masked != text {
                    let before = reference_counts(text);
                    for (id, count) in reference_counts(&masked) {
                        let added =
                            count.saturating_sub(before.get(&id).copied().unwrap_or_default());
                        if added > 0 {
                            *redacted.entry(id).or_default() += added;
                        }
                    }
                }
                Ok(masked)
            },
            false,
            true,
        )?;
        if !entries.is_empty() && (entries.len() != count || self.active()) {
            self.save(&entries)?;
        }
        drop(entries);
        drop(_lock);
        self.notify_operations("redact", "Outbound request", redacted);
        Ok(())
    }
    fn restore_text(&self, text: &str, strict: bool) -> AppResult<String> {
        self.restore_text_impl(text, strict, "Response text", true)
    }
    fn restore_text_at(&self, text: &str, strict: bool, location: &str) -> AppResult<String> {
        self.restore_text_impl(text, strict, location, true)
    }
    fn restore(&self, value: &mut Value, strict: bool) -> AppResult<()> {
        visit(
            value,
            &mut |text, executable| {
                self.restore_text_at(
                    text,
                    strict || executable,
                    if strict || executable {
                        "Tool arguments"
                    } else {
                        "Response text"
                    },
                )
            },
            !strict,
        )?;
        Ok(())
    }
    fn validate_tool(&self, name: Option<&str>, arguments: &Value) -> AppResult<()> {
        self.contract
            .lock()
            .map_err(|_| error())?
            .tool(name, arguments)
    }
    fn validate_output(&self, text: &str) -> AppResult<()> {
        self.contract.lock().map_err(|_| error())?.output(text)
    }
    fn validate_response(&self, value: &Value) -> AppResult<()> {
        self.contract.lock().map_err(|_| error())?.response(value)
    }
    fn structured_output(&self) -> bool {
        self.contract.lock().is_ok_and(|c| c.structured())
    }
    fn scanned(&self) -> bool {
        self.scanned.load(Ordering::Acquire)
    }
    fn mark_scanned(&self) {
        self.scanned.store(true, Ordering::Release);
    }
    fn active(&self) -> bool {
        self.allowed.lock().is_ok_and(|s| !s.is_empty())
    }
}

/// Walk content values without touching protocol identities or signed state.
/// Arguments are JSON *inside* a JSON string and need their own parse/serialize.
pub fn visit(
    value: &mut Value,
    f: &mut dyn FnMut(&str, bool) -> AppResult<String>,
    response: bool,
) -> AppResult<()> {
    visit_with_mode(value, f, response, response)
}
fn visit_with_mode(
    value: &mut Value,
    f: &mut dyn FnMut(&str, bool) -> AppResult<String>,
    response: bool,
    opaque: bool,
) -> AppResult<()> {
    match value {
        Value::String(s) => *s = f(s, false)?,
        Value::Array(items) => {
            for item in items {
                visit_with_mode(item, f, response, opaque)?;
            }
        }
        Value::Object(map) => {
            if response
                && ["signature", "thought_signature"].iter().any(|key| {
                    map.get(*key)
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.is_empty())
                })
            {
                return Ok(());
            }
            let protocol_object = map.contains_key("choices")
                || map.contains_key("role")
                || map.contains_key("object")
                || map.get("type").and_then(Value::as_str).is_some_and(|kind| {
                    matches!(
                        kind,
                        "message"
                            | "function_call"
                            | "custom_tool_call"
                            | "function"
                            | "text"
                            | "output_text"
                            | "thinking"
                            | "reasoning"
                            | "compaction"
                            | "compaction_summary"
                    ) || kind.starts_with("response.")
                        || kind.starts_with("content_block")
                })
                || (map.contains_key("name") && map.contains_key("arguments"));
            let mut replaced = serde_json::Map::new();
            for (key, mut item) in std::mem::take(map) {
                if opaque
                    && protocol_object
                    && matches!(
                        key.as_str(),
                        "encrypted_content" | "signature" | "thought_signature"
                    )
                {
                    replaced.insert(key, item);
                    continue;
                }
                if response
                    && protocol_object
                    && matches!(
                        key.as_str(),
                        "id" | "call_id"
                            | "item_id"
                            | "model"
                            | "type"
                            | "object"
                            | "role"
                            | "signature"
                            | "encrypted_content"
                            | "finish_reason"
                            | "stop_reason"
                            | "status"
                            | "name"
                    )
                {
                    replaced.insert(key, item);
                    continue;
                }
                let executable = matches!(key.as_str(), "arguments" | "partial_json" | "input");
                let nested = executable || matches!(key.as_str(), "content" | "text" | "output");
                let mut decoded = false;
                if nested {
                    if let Some(text) = item.as_str() {
                        if let Ok(mut inner) = serde_json::from_str::<Value>(text) {
                            if inner.is_object() || inner.is_array() {
                                let before = inner.clone();
                                visit(&mut inner, &mut |s, _| f(s, executable), false)?;
                                if inner != before {
                                    item = Value::String(
                                        serde_json::to_string(&inner).map_err(|_| error())?,
                                    );
                                }
                                decoded = true;
                            }
                        }
                    }
                }
                if !decoded {
                    visit_with_mode(
                        &mut item,
                        &mut |s, exec| f(s, executable || exec),
                        response && !executable,
                        opaque && !executable,
                    )?;
                }
                let new_key = if response { key } else { f(&key, false)? };
                if replaced.insert(new_key, item).is_some() {
                    return Err(AppError::InvalidParams(
                        "Secret restoration produced a duplicate JSON key".into(),
                    ));
                }
            }
            *map = replaced;
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_api_keys::MockKeychain;
    fn fixture() -> Vault {
        Vault::new(Arc::new(MockKeychain::new()), "client", "session", false)
    }
    fn resolve(id: &str) -> AppResult<Option<String>> {
        Ok((id == "abc123").then(|| "the original 🔐\n\"\\".into()))
    }
    #[test]
    fn accepted_variants() {
        for tag in [
            marker("abc123"),
            "<LOCALROUTER-REDACTED ID=ABC123>".into(),
            "<LocalRouter-Redacted usage='anything > works'\n ID = 'AbC123' extra=ignored />"
                .into(),
            "<localrouter-redacted id=abc123>".into(),
        ] {
            assert_eq!(
                restore_text(&tag, true, resolve).unwrap(),
                resolve("abc123").unwrap().unwrap()
            );
        }
    }
    #[test]
    fn rejects_ambiguous_or_non_tags() {
        for tag in [
            "localrouter-redacted id=abc123",
            "< localrouter-redacted id=abc123>",
            "</localrouter-redacted id=abc123>",
            "<localrouter-redaceted id=abc123>",
            "<localrouter-redacted-more id=abc123>",
            "<localrouter-redacted id='abc123' ID=abc123>",
            "<localrouter-redacted id='abc123 >",
            "<localrouter-redacted id=' abc123'>",
            "<localrouter-redacted id='abc123x'>",
            "<localrouter-redacted usage='id=abc123'>",
            "<localrouter-redacted id=''>",
            "<localrouter-redacted id=abc123/",
            "&lt;localrouter-redacted id=abc123&gt;",
            "<unrelated id=abc123>localrouter-redacted</unrelated>",
        ] {
            assert_eq!(restore_text(tag, false, resolve).unwrap(), tag, "{tag}");
        }
    }
    #[test]
    fn unicode_near_markers_does_not_panic() {
        for input in [
            "<éééééééééééééé>",
            "<localrouter-🔐redacted>",
            "💫 <LOCALROUTER-REDACTED ID=ABC123> 日本語",
        ] {
            let mut stream = TextStream::default();
            let a = stream
                .push(input, false, |s| restore_text(s, false, resolve))
                .unwrap();
            let b = stream
                .push("", true, |s| restore_text(s, false, resolve))
                .unwrap();
            assert_eq!(a + b.as_str(), restore_text(input, false, resolve).unwrap());
        }
    }
    #[test]
    fn every_split_and_character_fragmentation() {
        for tag in [
            marker("abc123"),
            "<LOCALROUTER-REDACTED extra='> not end' ID='ABC123'>".into(),
        ] {
            let input = format!("before {tag} middle {tag} after");
            for split in 0..=input.len() {
                let mut stream = TextStream::default();
                let mut output = stream
                    .push(&input[..split], false, |s| restore_text(s, false, resolve))
                    .unwrap();
                output += &stream
                    .push(&input[split..], true, |s| restore_text(s, false, resolve))
                    .unwrap();
                assert_eq!(
                    output,
                    restore_text(&input, false, resolve).unwrap(),
                    "split {split}"
                );
            }
            let mut stream = TextStream::default();
            let mut output = String::new();
            for ch in input.chars() {
                output += &stream
                    .push(&ch.to_string(), false, |s| restore_text(s, false, resolve))
                    .unwrap();
            }
            output += &stream
                .push("", true, |s| restore_text(s, false, resolve))
                .unwrap();
            assert_eq!(output, restore_text(&input, false, resolve).unwrap());
        }
    }
    #[test]
    fn durable_recovery_and_scope_authorization() {
        let store = Arc::new(MockKeychain::new());
        let a = Vault::new(store.clone(), "a", "same", false);
        a.register("synthetic-secret").unwrap();
        let mut protected = serde_json::json!({"content":"synthetic-secret"});
        a.protect(&mut protected).unwrap();
        let canonical = protected.clone();
        drop(a);
        let recovered = Vault::new(store.clone(), "a", "same", false);
        assert!(
            recovered.restore(&mut protected.clone(), true).is_err(),
            "must not resolve references before request authorization"
        );
        recovered.protect(&mut protected).unwrap();
        assert_eq!(protected, canonical);
        recovered.restore(&mut protected, true).unwrap();
        assert_eq!(protected["content"], "synthetic-secret");
        for (owner, session) in [("b", "same"), ("a", "different")] {
            let other = Vault::new(store.clone(), owner, session, false);
            other.protect(&mut canonical.clone()).unwrap();
            assert!(other.restore(&mut canonical.clone(), true).is_err());
        }
        recovered.forget().unwrap();
        assert!(recovered.restore(&mut canonical.clone(), true).is_err());
    }
    #[test]
    fn stable_masking_json_and_overlap() {
        let vault = fixture();
        for s in ["abc", "bcdef", "another secret"] {
            vault.register(s).unwrap();
        }
        let mut value = serde_json::json!({"content":"abcdef and another secret", "arguments":"{\"x\":\"another secret\"}"});
        let original = value.clone();
        vault.protect(&mut value).unwrap();
        assert!(!value.to_string().contains("another secret"));
        let mut again = original.clone();
        vault.protect(&mut again).unwrap();
        assert_eq!(value, again);
        vault.restore(&mut value, true).unwrap();
        assert_eq!(value, original);
    }
    #[test]
    fn originals_are_not_rescanned() {
        let secret = marker("abc123");
        let output = restore_text(&marker("abc123"), true, |_| Ok(Some(secret.clone()))).unwrap();
        assert_eq!(output, secret);
    }
    #[test]
    fn randomized_round_trip_corpus() {
        let vault = fixture();
        for i in 0..128 {
            let secret = format!("corpus-{i:04x}-é\n\r\"\\🦀");
            vault.register(&secret).unwrap();
            let original = serde_json::json!({"content":format!("before {secret} after"),"arguments":serde_json::json!({"key":secret}).to_string()});
            let mut value = original.clone();
            vault.protect(&mut value).unwrap();
            vault.restore(&mut value, true).unwrap();
            assert_eq!(value, original);
        }
    }
    #[test]
    fn literal_tag_mentions_in_executable_content_are_preserved() {
        for literal in [
            "'<localrouter-redacted' in actual",
            "\"<LOCALROUTER-REDACTED\" in actual",
            "<localrouter-redacted",
            "<localrouter-redactedness>",
        ] {
            for (text, expected) in [
                (
                    format!("{}; {literal}", marker("abc123")),
                    format!("original; {literal}"),
                ),
                (
                    format!("{literal}; {}", marker("abc123")),
                    format!("{literal}; original"),
                ),
            ] {
                let restored = restore_text(&text, true, |_| Ok(Some("original".into()))).unwrap();
                assert_eq!(restored, expected);
                let encoded = serde_json::json!({"code": text}).to_string();
                let restored =
                    restore_text(&encoded, true, |_| Ok(Some("original".into()))).unwrap();
                assert_eq!(
                    serde_json::from_str::<Value>(&restored).unwrap()["code"],
                    expected
                );
            }
        }
        for malformed in [
            "<localrouter-redacted>",
            "<localrouter-redacted/>",
            "<localrouter-redacted id=",
            "<localrouter-redacted id='unknown'/>",
        ] {
            assert!(restore_text(malformed, true, |_| Ok(None)).is_err());
            assert!(restore_text(
                &format!("'<localrouter-redacted' in actual; {malformed}"),
                true,
                |_| Ok(None),
            )
            .is_err());
        }
    }
    #[test]
    fn unknown_executable_tags_fail() {
        assert!(restore_text("<LOCALROUTER-REDACTED id=unknown>", true, resolve).is_err());
        assert!(restore_text("<localrouter-redacted id=abc123", true, resolve).is_err());
    }
    #[test]
    fn native_store_failure_blocks() {
        struct Failed;
        impl KeychainStorage for Failed {
            fn store(&self, _: &str, _: &str, _: &str) -> AppResult<()> {
                Err(storage_error())
            }
            fn get(&self, _: &str, _: &str) -> AppResult<Option<String>> {
                Ok(None)
            }
            fn delete(&self, _: &str, _: &str) -> AppResult<()> {
                Ok(())
            }
        }
        let vault = Vault::new(Arc::new(Failed), "owner", "session", false);
        assert!(vault.register("must not dispatch").is_err());
    }
    struct NativeProofStore(String);
    impl KeychainStorage for NativeProofStore {
        fn store(&self, service: &str, account: &str, secret: &str) -> AppResult<()> {
            lr_api_keys::keychain_trait::SystemKeychain.store(
                &format!("{service}-{}", self.0),
                account,
                secret,
            )
        }
        fn get(&self, service: &str, account: &str) -> AppResult<Option<String>> {
            lr_api_keys::keychain_trait::SystemKeychain
                .get(&format!("{service}-{}", self.0), account)
        }
        fn delete(&self, service: &str, account: &str) -> AppResult<()> {
            lr_api_keys::keychain_trait::SystemKeychain
                .delete(&format!("{service}-{}", self.0), account)
        }
    }
    #[test]
    #[ignore = "Explicit native keychain proof; creates and deletes only a synthetic isolated credential"]
    fn native_os_store_round_trip() {
        let session = format!("native-proof-{}-{}", std::process::id(), now());
        let owner = format!("synthetic-test-owner-{session}");
        let store = Arc::new(NativeProofStore(session.clone()));
        let vault = Arc::new(Vault::new(store.clone(), &owner, &session, false));
        vault.register("synthetic-native-secret-🔐").unwrap();
        let mut value = serde_json::json!({"content":"synthetic-native-secret-🔐"});
        vault.protect(&mut value).unwrap();
        let mut restorer = crate::wire::ResponseRestorer::new(vault.clone());
        restorer.json(serde_json::json!({"object":"response","id":"opaque-native-process-proof","output":[]}).to_string().as_bytes()).unwrap();
        drop(restorer);
        let proof = serde_json::json!({"owner":owner,"tag":value["content"],"namespace":session});
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "placeholders::tests::native_os_store_child_process",
                "--nocapture",
            ])
            .env("LOCALROUTER_SYNTHETIC_VAULT_PROOF", proof.to_string())
            .status()
            .unwrap();
        assert!(
            status.success(),
            "Fresh process must recover the OS-backed response scope"
        );
        drop(vault);
        let recovered = Vault::new(store.clone(), &owner, &session, false);
        recovered.protect(&mut value).unwrap();
        recovered.restore(&mut value, true).unwrap();
        assert_eq!(value["content"], "synthetic-native-secret-🔐");
        recovered.forget().unwrap();
        recovered
            .storage
            .delete(SERVICE, &recovered.index_key())
            .unwrap();
        store.delete(SERVICE, OWNERS).unwrap();
    }
    #[test]
    #[ignore = "Invoked only by the isolated native OS store proof"]
    fn native_os_store_child_process() {
        let Ok(proof) = std::env::var("LOCALROUTER_SYNTHETIC_VAULT_PROOF") else {
            return;
        };
        let proof: Value = serde_json::from_str(&proof).unwrap();
        let vault = Vault::continue_response(
            Arc::new(NativeProofStore(
                proof["namespace"].as_str().unwrap().into(),
            )),
            proof["owner"].as_str().unwrap(),
            "opaque-native-process-proof",
        )
        .unwrap();
        assert_eq!(
            vault
                .restore_text(proof["tag"].as_str().unwrap(), true)
                .unwrap(),
            "synthetic-native-secret-🔐"
        );
    }
}

#[cfg(test)]
mod robustness_tests {
    use super::*;
    #[test]
    fn adversarial_parser_and_stream_corpus() {
        let alphabet = [
            '<', '>', '/', '\'', '"', ' ', '=', 'I', 'd', 'a', '1', 'é', '🦀', '\n',
        ];
        let mut state = 0x973ba229u64;
        for _ in 0..2000 {
            let mut input = String::new();
            for _ in 0..128 {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                input.push(alphabet[(state >> 32) as usize % alphabet.len()]);
            }
            let expected = restore_text(&input, false, |_| Ok(None)).unwrap();
            let mut stream = TextStream::default();
            let mut actual = String::new();
            for ch in input.chars() {
                actual += &stream
                    .push(&ch.to_string(), false, |s| {
                        restore_text(s, false, |_| Ok(None))
                    })
                    .unwrap();
            }
            actual += &stream
                .push("", true, |s| restore_text(s, false, |_| Ok(None)))
                .unwrap();
            assert_eq!(actual, expected);
        }
    }
    #[test]
    fn authored_keys_and_id_values_restore() {
        let vault = Vault::new(
            Arc::new(lr_api_keys::MockKeychain::new()),
            "test",
            "keys",
            false,
        );
        vault.register("secret-key-value").unwrap();
        let mut protected = serde_json::json!({"content":"secret-key-value"});
        vault.protect(&mut protected).unwrap();
        let tag = protected["content"].as_str().unwrap();
        let mut args = serde_json::json!({"id":tag,tag:"value"});
        vault.restore(&mut args, true).unwrap();
        assert_eq!(args["id"], "secret-key-value");
        assert_eq!(args["secret-key-value"], "value");
    }
    #[test]
    fn durable_response_owner_binding() {
        let store = Arc::new(lr_api_keys::MockKeychain::new());
        let vault = Vault::new(store.clone(), "client", "scope", false);
        vault.register("the-secret").unwrap();
        let mut v = Value::String("the-secret".into());
        vault.protect(&mut v).unwrap();
        vault.bind_response("resp_test").unwrap();
        drop(vault);
        let recovered = Vault::continue_response(store.clone(), "client", "resp_test").unwrap();
        recovered.restore(&mut v, true).unwrap();
        assert_eq!(v, "the-secret");
        assert!(Vault::continue_response(store.clone(), "other", "resp_test").is_err());
        drop(recovered);
        Vault::forget_client(store.clone(), "client").unwrap();
        assert!(Vault::continue_response(store, "client", "resp_test").is_err());
    }
    #[test]
    fn cleanup_pins_active_scopes() {
        let store = Arc::new(lr_api_keys::MockKeychain::new());
        let vault = Vault::new(store.clone(), "cleanup-only-owner", "scope", false);
        vault.register("secret").unwrap();
        Vault::cleanup_client(store.clone(), "cleanup-only-owner", 0).unwrap();
        assert!(vault.knows("secret").unwrap());
        drop(vault);
        Vault::cleanup_client(store.clone(), "cleanup-only-owner", 0).unwrap();
        let recovered = Vault::new(store, "cleanup-only-owner", "scope", false);
        assert!(!recovered.knows("secret").unwrap());
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    #[test]
    fn owned_session_restores_after_scanning_is_disabled_without_masking_new_data() {
        let store = Arc::new(lr_api_keys::MockKeychain::new());
        let source = Vault::new(store.clone(), "restore-only-owner", "conversation", false);
        source.register("synthetic secret").unwrap();
        let mut marker = Value::String("synthetic secret".into());
        source.protect(&mut marker).unwrap();
        drop(source);
        assert!(
            Vault::find_session(store.clone(), "other-owner", "conversation")
                .unwrap()
                .is_none()
        );
        assert!(
            Vault::find_session(store.clone(), "restore-only-owner", "unrelated")
                .unwrap()
                .is_none()
        );
        let vault = Vault::find_scope(
            store,
            "restore-only-owner",
            Some("untracked-provider-id"),
            Some("conversation"),
        )
        .unwrap()
        .unwrap();
        vault.set_restore_only();
        let mut outgoing = serde_json::json!({"content":"synthetic secret"});
        vault.protect(&mut outgoing).unwrap();
        assert_eq!(outgoing["content"], "synthetic secret");
        vault.restore(&mut marker, true).unwrap();
        assert_eq!(marker, "synthetic secret");
        let unknown = "<localrouter-redacted id=example/>";
        assert_eq!(
            vault.restore_text(unknown, true).unwrap(),
            unknown,
            "Off permits literal examples while restoring matching owned IDs"
        );
        vault.set_request_contract(&serde_json::json!({"tools":[{"type":"function","name":"write","strict":true,"parameters":{"type":"object","properties":{"content":{"enum":["synthetic secret"]}}}}]})).unwrap();
        vault
            .validate_tool(
                Some("write"),
                &serde_json::json!({"content":"synthetic secret"}),
            )
            .unwrap();
        assert!(vault
            .validate_tool(Some("write"), &serde_json::json!({"content":"wrong"}))
            .is_err());
    }
    #[test]
    fn unknown_expired_and_foreign_response_ids_grant_no_capability() {
        let store = Arc::new(lr_api_keys::MockKeychain::new());
        assert!(
            Vault::find_response(store.clone(), "owner", "before-redaction")
                .unwrap()
                .is_none()
        );
        let vault = Vault::new(store.clone(), "owner", "scope", false);
        vault.register("synthetic secret").unwrap();
        vault.bind_response("opaque/id:ABC").unwrap();
        assert!(
            Vault::find_response(store.clone(), "other-owner", "opaque/id:ABC")
                .unwrap()
                .is_none()
        );
        let account = vault.account.clone();
        drop(vault);
        store.delete(SERVICE, &account).unwrap();
        assert!(Vault::find_response(store, "owner", "opaque/id:ABC")
            .unwrap()
            .is_none());
    }
    #[test]
    fn response_lookup_does_not_hide_revocation_corruption_or_storage_failure() {
        let store = Arc::new(lr_api_keys::MockKeychain::new());
        let vault = Vault::new(store.clone(), "revoked-owner", "scope", false);
        vault.bind_response("opaque").unwrap();
        drop(vault);
        Vault::forget_client(store.clone(), "revoked-owner").unwrap();
        assert!(Vault::find_response(store.clone(), "revoked-owner", "unknown").is_err());
        store
            .store(
                SERVICE,
                &format!("index-{}", key("corrupt", "")),
                r#"{"responses":[]}"#,
            )
            .unwrap();
        assert!(Vault::find_response(store, "corrupt", "unknown").is_err());
        struct Unavailable;
        impl KeychainStorage for Unavailable {
            fn store(&self, _: &str, _: &str, _: &str) -> AppResult<()> {
                Err(storage_error())
            }
            fn get(&self, _: &str, _: &str) -> AppResult<Option<String>> {
                Err(storage_error())
            }
            fn delete(&self, _: &str, _: &str) -> AppResult<()> {
                Err(storage_error())
            }
        }
        assert!(Vault::find_response(Arc::new(Unavailable), "owner", "unknown").is_err());
    }
    #[test]
    fn empty_responses_scopes_can_continue_before_the_first_secret() {
        let store = Arc::new(lr_api_keys::MockKeychain::new());
        let vault = Vault::new(store.clone(), "owner", "scope", false);
        vault.bind_response("resp_empty").unwrap();
        drop(vault);
        let recovered = Vault::continue_response(store, "owner", "resp_empty").unwrap();
        recovered.register("first secret").unwrap();
        let mut content = Value::String("first secret".into());
        recovered.protect(&mut content).unwrap();
        recovered.restore(&mut content, true).unwrap();
        assert_eq!(content, "first secret");
    }
    #[test]
    fn failed_deletion_revokes_other_live_capabilities() {
        struct FailingDelete(lr_api_keys::MockKeychain);
        impl KeychainStorage for FailingDelete {
            fn get(&self, s: &str, k: &str) -> AppResult<Option<String>> {
                self.0.get(s, k)
            }
            fn store(&self, s: &str, k: &str, v: &str) -> AppResult<()> {
                self.0.store(s, k, v)
            }
            fn delete(&self, _: &str, _: &str) -> AppResult<()> {
                Err(storage_error())
            }
        }
        let store = Arc::new(FailingDelete(lr_api_keys::MockKeychain::new()));
        let first = Vault::new(store.clone(), "owner", "scope", false);
        first.register("original").unwrap();
        let mut value = Value::String("original".into());
        first.protect(&mut value).unwrap();
        let other = Vault::new(store, "owner", "scope", false);
        other.protect(&mut value).unwrap();
        assert!(first.forget().is_err());
        assert!(other.restore(&mut value, true).is_err());
    }
}

#[cfg(test)]
mod owner_cleanup_tests {
    use super::*;
    use lr_api_keys::MockKeychain;
    use std::sync::atomic::AtomicBool;
    struct FlakyDelete {
        inner: MockKeychain,
        fail: AtomicBool,
    }
    impl KeychainStorage for FlakyDelete {
        fn store(&self, s: &str, a: &str, v: &str) -> AppResult<()> {
            self.inner.store(s, a, v)
        }
        fn get(&self, s: &str, a: &str) -> AppResult<Option<String>> {
            self.inner.get(s, a)
        }
        fn delete(&self, s: &str, a: &str) -> AppResult<()> {
            if self.fail.load(Ordering::Relaxed) {
                return Err(storage_error());
            }
            self.inner.delete(s, a)
        }
    }
    #[test]
    fn removed_owner_cleanup_retries_after_restart_and_revokes_active_capabilities() {
        let store = Arc::new(FlakyDelete {
            inner: MockKeychain::new(),
            fail: AtomicBool::new(false),
        });
        let vault = Vault::new(store.clone(), "removed-owner", "conversation", false);
        vault.register("synthetic cleanup original").unwrap();
        vault.bind_response("resp_removed").unwrap();
        let account = vault.account.clone();
        store.fail.store(true, Ordering::Relaxed);
        assert!(Vault::forget_client(store.clone(), "removed-owner").is_err());
        assert!(vault.knows("synthetic cleanup original").is_err());
        drop(vault);
        assert!(Vault::continue_response(store.clone(), "removed-owner", "resp_removed").is_err());
        store.fail.store(false, Ordering::Relaxed);
        // No owner remains in application settings. The OS metadata registry
        // still identifies the failed cleanup after all request objects dropped.
        Vault::cleanup_registered(store.clone(), &[], 3600).unwrap();
        assert!(store.get(SERVICE, &account).unwrap().is_none());
        assert!(Vault::continue_response(store, "removed-owner", "resp_removed").is_err());
    }
}

#[cfg(test)]
mod opaque_history_tests {
    use super::*;
    use lr_api_keys::MockKeychain;
    use serde_json::json;
    #[test]
    fn exact_compaction_or_reasoning_state_recovers_only_its_owned_scope_after_restart() {
        for kind in ["compaction", "compaction_summary", "reasoning"] {
            let storage = Arc::new(MockKeychain::new());
            let vault = Arc::new(Vault::new(
                storage.clone(),
                "opaque-owner",
                "explicit-conversation",
                false,
            ));
            vault.register("synthetic opaque original").unwrap();
            let mut input = json!({"input":"synthetic opaque original"});
            vault.protect(&mut input).unwrap();
            let tag = input["input"].as_str().unwrap().to_owned();
            let response =
                json!({"output":[{"type":kind,"encrypted_content":"provider-owned-opaque-bytes"}]});
            let mut codec = crate::wire::ResponseRestorer::new(vault);
            assert_eq!(
                serde_json::from_slice::<Value>(
                    &codec.json(response.to_string().as_bytes()).unwrap()
                )
                .unwrap(),
                response
            );
            drop(codec);
            let resumed = Vault::new(
                storage.clone(),
                "opaque-owner",
                "explicit-conversation",
                false,
            );
            let mut replay = json!({"input":response["output"]});
            resumed.protect(&mut replay).unwrap();
            assert_eq!(replay["input"], response["output"]);
            assert_eq!(
                resumed.restore_text(&tag, true).unwrap(),
                "synthetic opaque original"
            );
            for (owner, scope, opaque) in [
                (
                    "other-client",
                    "explicit-conversation",
                    "provider-owned-opaque-bytes",
                ),
                (
                    "opaque-owner",
                    "other-conversation",
                    "provider-owned-opaque-bytes",
                ),
                (
                    "opaque-owner",
                    "explicit-conversation",
                    "modified-opaque-bytes",
                ),
            ] {
                let other = Vault::new(storage.clone(), owner, scope, false);
                let mut input = json!({"input":[{"type":kind,"encrypted_content":opaque}]});
                other.protect(&mut input).unwrap();
                assert!(other.restore_text(&tag, true).is_err());
            }
            resumed.forget().unwrap();
            let mut replay = json!({"input":response["output"]});
            resumed.protect(&mut replay).unwrap();
            assert!(resumed.restore_text(&tag, true).is_err());
        }
    }
}

#[cfg(test)]
mod reasoning_summary_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn visible_reasoning_summary_round_trips_while_encrypted_state_stays_exact() {
        let vault = Arc::new(Vault::new(
            Arc::new(lr_api_keys::MockKeychain::new()),
            "summary",
            "conversation",
            false,
        ));
        vault.register("synthetic reasoning original").unwrap();
        let mut input = json!({"input":"synthetic reasoning original"});
        vault.protect(&mut input).unwrap();
        let tag = input["input"].as_str().unwrap();
        let response = json!({"output":[{"type":"reasoning","encrypted_content":"opaque-bytes","summary":[{"type":"summary_text","text":tag}]}]});
        let mut codec = crate::wire::ResponseRestorer::new(vault.clone());
        let response: Value =
            serde_json::from_slice(&codec.json(response.to_string().as_bytes()).unwrap()).unwrap();
        assert_eq!(response["output"][0]["encrypted_content"], "opaque-bytes");
        assert_eq!(
            response["output"][0]["summary"][0]["text"],
            "synthetic reasoning original"
        );
        let mut replay = json!({"input":response["output"]});
        vault.protect(&mut replay).unwrap();
        assert!(!replay.to_string().contains("synthetic reasoning original"));
        assert_eq!(replay["input"][0]["encrypted_content"], "opaque-bytes");
    }
}

#[cfg(test)]
mod monitor_mask_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn diagnostic_masking_never_rescans_inserted_placeholders() {
        let vault = Vault::for_client(
            Arc::new(lr_api_keys::MockKeychain::new()),
            "overlapping-monitor",
        );
        vault.register("verbatim").unwrap();
        vault.register("long synthetic credential").unwrap();
        let mut value = json!({"content":"long synthetic credential then verbatim"});
        vault.redact_monitor(&mut value).unwrap();
        let masked = value.clone();
        vault.redact_monitor(&mut value).unwrap();
        assert_eq!(value, masked);
        let text = value["content"].as_str().unwrap();
        assert_eq!(text.matches("<localrouter-redacted").count(), 2);
        assert_eq!(
            text.matches("usage='Unmasked when returned verbatim'/>")
                .count(),
            2
        );
    }

    #[test]
    fn diagnostic_masking_retains_payload_and_does_not_authorize_replay() {
        let vault = Vault::new(
            Arc::new(lr_api_keys::MockKeychain::new()),
            "monitor",
            "scope",
            false,
        );
        let secret = "synthetic \"quoted\"\ncredential";
        vault.register(secret).unwrap();
        let mut data = json!({"data":{"request_body":{"content":secret},"raw_request":json!({"content":secret}).to_string(),"error":format!("upstream failed: {secret}")},"client_name":"Codex","session_id":"request","conversation":{"id":"conversation","title":secret}});
        vault.redact_monitor(&mut data).unwrap();
        let serialized = data.to_string();
        assert!(!serialized.contains(
            &serde_json::to_string(secret).unwrap()
                [1..serde_json::to_string(secret).unwrap().len() - 1]
        ));
        assert_eq!(data["client_name"], "Codex");
        assert_eq!(data["session_id"], "request");
        assert_eq!(data["conversation"]["id"], "conversation");
        assert!(data["data"]["error"]
            .as_str()
            .unwrap()
            .starts_with("upstream failed: <localrouter-redacted"));
        let raw: Value =
            serde_json::from_str(data["data"]["raw_request"].as_str().unwrap()).unwrap();
        assert_eq!(raw["content"], data["data"]["request_body"]["content"]);
        assert!(vault
            .restore_text(raw["content"].as_str().unwrap(), true)
            .is_err());
        vault.set_restore_only();
        let mut allowed = json!({"content":secret});
        vault.redact_monitor(&mut allowed).unwrap();
        assert_eq!(allowed["content"], secret);
    }
}

#[cfg(test)]
mod client_map_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn client_migration_preserves_ids_and_isolates_other_clients() {
        let storage = Arc::new(lr_api_keys::MockKeychain::new());
        let owner = "client-map-migration-test".to_owned();
        let mut tags = Vec::new();
        for scope in ["conversation-one", "conversation-two"] {
            let legacy = Vault::new(storage.clone(), &owner, scope, false);
            let secret = format!("synthetic-secret-{scope}");
            legacy.register(&secret).unwrap();
            let mut value = json!({"content":secret});
            legacy.protect(&mut value).unwrap();
            legacy.bind_response(scope).unwrap();
            tags.push((value["content"].as_str().unwrap().to_owned(), secret));
        }
        let client = Vault::find_client(storage.clone(), &owner, RETENTION_SECS)
            .unwrap()
            .unwrap();
        for (tag, secret) in &tags {
            assert_eq!(client.restore_text(tag, true).unwrap(), *secret);
        }
        drop(client);
        let restarted = Vault::find_client(storage.clone(), &owner, RETENTION_SECS)
            .unwrap()
            .unwrap();
        for (tag, secret) in &tags {
            assert_eq!(restarted.restore_text(tag, true).unwrap(), *secret);
            let mut request = json!({"content":secret});
            restarted.protect(&mut request).unwrap();
            assert_eq!(request["content"], *tag);
        }
        let foreign = Vault::for_client(storage.clone(), "foreign-client");
        for (tag, _) in &tags {
            assert!(foreign.restore_text(tag, true).is_err());
        }
        assert!(
            Vault::find_client(storage, "foreign-client", RETENTION_SECS)
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn expired_client_maps_are_removed_on_access_and_monitor_does_not_pin_them() {
        let storage = Arc::new(lr_api_keys::MockKeychain::new());
        let owner = "client-map-expiry-test".to_owned();
        let client = Vault::for_client(storage.clone(), &owner);
        client.register("synthetic-expiring-secret").unwrap();
        let monitor = client.monitor_context().unwrap();
        let account = client.account.clone();
        let mut index = client.index().unwrap();
        index["scopes"][&account] = Value::from(now() - RETENTION_SECS - 1);
        client.save_index(&index).unwrap();
        drop(client);
        assert!(Vault::find_client(storage.clone(), &owner, RETENTION_SECS)
            .unwrap()
            .is_none());
        assert!(storage.get(SERVICE, &account).unwrap().is_none());
        drop(monitor);
    }
    #[test]
    fn nested_json_and_unicode_escapes_restore_without_corrupting_syntax() {
        let vault = Vault::for_client(Arc::new(lr_api_keys::MockKeychain::new()), "escaped-client");
        let secret = "synthetic \"quotes\" and 'apostrophe' \\ path\r\n🔐";
        vault.register(secret).unwrap();
        let mut protected = json!({"content":secret});
        vault.protect(&mut protected).unwrap();
        let canonical = protected["content"].as_str().unwrap();
        assert!(canonical.contains("id='"));
        for tag in [
            canonical.to_owned(),
            canonical
                .replace("id='", "ID='")
                .replace("localrouter-redacted", "LOCALROUTER-REDACTED"),
            canonical.replace("'", "\""),
        ] {
            for depth in 0..5 {
                let mut encoded = json!({"token":tag});
                let mut expected = json!({"token":secret});
                for _ in 0..depth {
                    encoded = Value::String(encoded.to_string());
                    expected = Value::String(expected.to_string());
                }
                let text = encoded.to_string();
                let restored = vault.restore_text(&text, true).unwrap();
                assert_eq!(serde_json::from_str::<Value>(&restored).unwrap(), expected);
            }
            let encoded = json!({"token":tag})
                .to_string()
                .replace('<', "\\u003c")
                .replace('>', "\\u003e");
            assert_eq!(
                serde_json::from_str::<Value>(&vault.restore_text(&encoded, true).unwrap())
                    .unwrap(),
                json!({"token":secret})
            );
        }
    }
}

#[cfg(test)]
mod store_management_tests {
    use super::*;
    use serde_json::json;
    const SECRET: &str = "synthetic-original-to-purge-1234567890";
    const OTHER: &str = "synthetic-original-to-keep-0987654321";
    #[test]
    fn inventory_never_touches_activity_and_purge_removes_legacy_aliases_only_for_its_owner() {
        let storage = Arc::new(lr_api_keys::MockKeychain::new());
        let owner = "store-management-owner";
        let mut old_tags = Vec::new();
        for scope in ["one", "two"] {
            let vault = Vault::new(storage.clone(), owner, scope, false);
            vault.register(SECRET).unwrap();
            let mut input = json!({"content":SECRET});
            vault.protect(&mut input).unwrap();
            old_tags.push(input["content"].as_str().unwrap().to_owned());
        }
        let active = Vault::find_client(storage.clone(), owner, RETENTION_SECS)
            .unwrap()
            .unwrap();
        active.register(OTHER).unwrap();
        let foreign = Vault::for_client(storage.clone(), "store-foreign-owner");
        foreign.register(SECRET).unwrap();
        let mut foreign_tag = json!({"content":SECRET});
        foreign.protect(&mut foreign_tag).unwrap();
        let mut index = active.index().unwrap();
        index["scopes"][&active.account] = json!(now() - 3600);
        active.save_index(&index).unwrap();
        let before = storage.get(SERVICE, &active.index_key()).unwrap();
        let entries = Vault::list_client(storage.clone(), owner).unwrap();
        assert_eq!(entries.len(), 3);
        assert!(!serde_json::to_string(&entries).unwrap().contains(SECRET));
        assert_eq!(storage.get(SERVICE, &active.index_key()).unwrap(), before);
        let id = candidate(&old_tags[0], 0).unwrap().1;
        assert_eq!(
            Vault::purge_client(storage.clone(), owner, Some(&id.to_ascii_uppercase())).unwrap(),
            2
        );
        assert!(active.restore_text(&old_tags[0], true).is_err());
        assert!(
            active.register(SECRET).is_err(),
            "old request must not recreate purged values"
        );
        let entries = Vault::list_client(storage.clone(), owner).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].last_used_at,
            index["scopes"][&active.account].as_u64().unwrap()
        );
        let recovered = Vault::find_client(storage.clone(), owner, RETENTION_SECS)
            .unwrap()
            .unwrap();
        for tag in &old_tags {
            assert!(recovered.restore_text(tag, true).is_err());
        }
        assert_eq!(
            recovered
                .restore_text(&entries[0].placeholder, true)
                .unwrap(),
            OTHER
        );
        assert_eq!(
            foreign
                .restore_text(foreign_tag["content"].as_str().unwrap(), true)
                .unwrap(),
            SECRET
        );
        assert_eq!(
            Vault::purge_client(
                storage.clone(),
                owner,
                Some(
                    &candidate(foreign_tag["content"].as_str().unwrap(), 0)
                        .unwrap()
                        .1
                )
            )
            .unwrap(),
            0
        );
        recovered.register(SECRET).unwrap();
        let mut fresh = json!({"content":SECRET});
        recovered.protect(&mut fresh).unwrap();
        assert!(!old_tags
            .iter()
            .any(|tag| tag == fresh["content"].as_str().unwrap()));
    }
    #[test]
    fn purge_failure_revokes_first_and_retry_allows_new_requests_without_reviving_old_vaults() {
        struct Failing {
            inner: lr_api_keys::MockKeychain,
            fail: AtomicBool,
        }
        impl KeychainStorage for Failing {
            fn get(&self, s: &str, a: &str) -> AppResult<Option<String>> {
                self.inner.get(s, a)
            }
            fn store(&self, s: &str, a: &str, v: &str) -> AppResult<()> {
                self.inner.store(s, a, v)
            }
            fn delete(&self, s: &str, a: &str) -> AppResult<()> {
                if self.fail.load(Ordering::Relaxed) {
                    Err(storage_error())
                } else {
                    self.inner.delete(s, a)
                }
            }
        }
        let storage = Arc::new(Failing {
            inner: lr_api_keys::MockKeychain::new(),
            fail: AtomicBool::new(false),
        });
        let active = Vault::for_client(storage.clone(), "purge-failure-owner");
        active.register(SECRET).unwrap();
        let mut value = json!({"content":SECRET});
        active.protect(&mut value).unwrap();
        storage.fail.store(true, Ordering::Relaxed);
        assert!(Vault::purge_client(storage.clone(), "purge-failure-owner", None).is_err());
        assert!(active
            .restore_text(value["content"].as_str().unwrap(), true)
            .is_err());
        assert!(
            Vault::find_client(storage.clone(), "purge-failure-owner", RETENTION_SECS).is_err()
        );
        assert!(Vault::list_client(storage.clone(), "purge-failure-owner").unwrap()[0].revoked);
        storage.fail.store(false, Ordering::Relaxed);
        assert_eq!(
            Vault::purge_client(storage.clone(), "purge-failure-owner", None).unwrap(),
            1
        );
        assert!(Vault::list_client(storage.clone(), "purge-failure-owner")
            .unwrap()
            .is_empty());
        assert!(active.register(SECRET).is_err());
        let fresh = Vault::for_client(storage, "purge-failure-owner");
        fresh.register(SECRET).unwrap();
        assert!(fresh
            .restore_text(value["content"].as_str().unwrap(), true)
            .is_err());
    }
    #[test]
    fn failed_single_secret_rewrite_revokes_the_original_and_preserves_other_entries() {
        struct Failing {
            inner: lr_api_keys::MockKeychain,
            account: String,
            fail: AtomicBool,
        }
        impl KeychainStorage for Failing {
            fn get(&self, s: &str, a: &str) -> AppResult<Option<String>> {
                self.inner.get(s, a)
            }
            fn store(&self, s: &str, a: &str, v: &str) -> AppResult<()> {
                if a == self.account && self.fail.load(Ordering::Relaxed) {
                    Err(storage_error())
                } else {
                    self.inner.store(s, a, v)
                }
            }
            fn delete(&self, s: &str, a: &str) -> AppResult<()> {
                self.inner.delete(s, a)
            }
        }
        let owner = "single-purge-write-failure";
        let storage = Arc::new(Failing {
            inner: lr_api_keys::MockKeychain::new(),
            account: key(owner, "client-secret-map-v2"),
            fail: AtomicBool::new(false),
        });
        let vault = Vault::for_client(storage.clone(), owner);
        vault.register(SECRET).unwrap();
        vault.register(OTHER).unwrap();
        let entries = vault.load().unwrap();
        let id = entries
            .iter()
            .find(|(_, value)| value.as_str() == SECRET)
            .unwrap()
            .0
            .clone();
        storage.fail.store(true, Ordering::Relaxed);
        assert!(Vault::purge_client(storage.clone(), owner, Some(&id)).is_err());
        let listed = Vault::list_client(storage.clone(), owner).unwrap();
        assert_eq!(listed.len(), 2);
        assert!(
            listed
                .iter()
                .find(|entry| entry.reference_id == id)
                .unwrap()
                .revoked
        );
        assert!(
            !listed
                .iter()
                .find(|entry| entry.reference_id != id)
                .unwrap()
                .revoked
        );
        let current = Vault::for_client(storage.clone(), owner);
        current.allowed.lock().unwrap().insert(id.clone());
        assert!(current.restore_text(&marker(&id), true).is_err());
        storage.fail.store(false, Ordering::Relaxed);
        assert_eq!(
            Vault::purge_client(storage.clone(), owner, Some(&id)).unwrap(),
            1
        );
        let raw = storage
            .inner
            .get(SERVICE, &storage.account)
            .unwrap()
            .unwrap();
        assert!(!raw.contains(SECRET));
        assert!(raw.contains(OTHER));
    }

    #[test]
    fn store_management_survives_file_restart_and_observes_expiry_without_extending_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("originals.json");
        let storage =
            Arc::new(lr_api_keys::keychain_trait::FileKeychain::new(path.clone()).unwrap());
        let vault = Vault::for_client(storage.clone(), "persistent-store-owner");
        vault.register(SECRET).unwrap();
        let inventory = Vault::list_client(storage.clone(), "persistent-store-owner").unwrap();
        drop(vault);
        drop(storage);
        let storage =
            Arc::new(lr_api_keys::keychain_trait::FileKeychain::new(path.clone()).unwrap());
        let restored = Vault::list_client(storage.clone(), "persistent-store-owner").unwrap();
        assert_eq!(inventory[0].placeholder, restored[0].placeholder);
        assert_eq!(inventory[0].expires_at, restored[0].expires_at);
        let vault = Vault::for_client(storage.clone(), "persistent-store-owner");
        let mut index = vault.index().unwrap();
        index["scopes"][&vault.account] = json!(now() - RETENTION_SECS - 1);
        vault.save_index(&index).unwrap();
        drop(vault);
        assert!(Vault::list_client(storage.clone(), "persistent-store-owner").unwrap()[0].expired);
        assert!(
            Vault::find_client(storage.clone(), "persistent-store-owner", RETENTION_SECS)
                .unwrap()
                .is_none()
        );
        assert!(
            Vault::list_client(storage.clone(), "persistent-store-owner")
                .unwrap()
                .is_empty()
        );
        assert!(!std::fs::read_to_string(path).unwrap().contains(SECRET));
    }
    #[test]
    fn operation_observers_report_only_committed_owned_substitutions_after_releasing_locks() {
        let storage = Arc::new(lr_api_keys::MockKeychain::new());
        let vault = Vault::for_client(storage.clone(), "operation-observer-owner");
        vault.register(SECRET).unwrap();
        let notices = Arc::new(Mutex::new(Vec::new()));
        let captured = notices.clone();
        vault.set_operation_observer(Arc::new(move |notice| {
            assert!(
                !Vault::list_client(storage.clone(), "operation-observer-owner")
                    .unwrap()
                    .is_empty()
            );
            captured.lock().unwrap().push(notice);
        }));
        let mut value = json!({"content":format!("{SECRET} {SECRET}")});
        vault.protect(&mut value).unwrap();
        assert_eq!(notices.lock().unwrap()[0].occurrences, 2);
        // Use an actual single reference rather than the display string containing two.
        let one = marker(&notices.lock().unwrap()[0].reference_id);
        vault.protect(&mut value).unwrap();
        assert_eq!(notices.lock().unwrap().len(), 1);
        assert_eq!(
            vault.restore_text_at(&one, true, "Tool arguments").unwrap(),
            SECRET
        );
        assert_eq!(notices.lock().unwrap()[1].action, "restore");
        assert_eq!(notices.lock().unwrap()[1].location, "Tool arguments");
        let before = notices.lock().unwrap().len();
        assert!(vault
            .restore_text(&format!("{one} <localrouter-redacted id='unknown'/>"), true)
            .is_err());
        assert_eq!(notices.lock().unwrap().len(), before);
        vault.set_request_contract(&json!({"tools":[{"type":"function","name":"write","parameters":{"type":"object","properties":{"value":{"enum":[one]}}}}]})).unwrap();
        assert_eq!(notices.lock().unwrap().len(), before);
        assert!(!format!("{:?}", notices.lock().unwrap()).contains(SECRET));
    }
}
