//! Self-describing secret references and scoped native credential storage.
//! Originals are serialized only into the OS credential store, never Debug.
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
pub const MAX_TAG_BYTES: usize = 16 * 1024 * 1024; // The existing gateway request/response body budget.
const SERVICE: &str = "LocalRouter-Redacted-Secrets-v1";
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
        "Secret placeholder could not be resolved in this authorized conversation".into(),
    )
}
fn parse_index(raw: &str) -> AppResult<Value> {
    let mut value: Value = serde_json::from_str(raw).map_err(|_| storage_error())?;
    let map = value.as_object_mut().ok_or_else(storage_error)?;
    for name in ["scopes", "responses", "revoked_scopes"] {
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
    format!("<localrouter-redacted id=\"{id}\" usage=\"Unmasked when returned verbatim\"/>")
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

/// Transform recognized references in one pass; do not recursively inspect inserted originals.
pub fn restore_text(
    text: &str,
    strict: bool,
    mut resolve: impl FnMut(&str) -> AppResult<Option<String>>,
) -> AppResult<String> {
    let mut out = String::new();
    let mut offset = 0;
    for (start, _) in text.match_indices('<') {
        if start < offset {
            continue;
        }
        if let Some((end, id)) = candidate(text, start) {
            out.push_str(&text[offset..start]);
            match resolve(&id)? {
                Some(value) => {
                    if out.len().saturating_add(value.len()) > MAX_TAG_BYTES {
                        return Err(AppError::InvalidParams(
                            "Restored content exceeds the gateway response budget".into(),
                        ));
                    }
                    out.push_str(&value);
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
                s.get(..TAG.len())
                    .is_some_and(|s| s.eq_ignore_ascii_case(TAG))
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
}
impl TextStream {
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
                    self.pending.push(ch);
                } else {
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
                    self.pending.push(ch);
                } else {
                    ready.push(ch);
                }
                continue;
            }
            if name_bytes == TAG.len() && !ch.is_ascii_whitespace() && ch != '/' && ch != '>' {
                ready.push_str(&std::mem::take(&mut self.pending));
                if ch == '<' {
                    self.pending.push(ch);
                } else {
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
                    ready.push_str(&restore(&self.pending)?);
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

/// A request capability over a durable owner/session record. Each request has
/// its own allowed-reference set; OS records are never searched globally.
type Detector = dyn Fn(&mut Value) -> AppResult<Vec<String>> + Send + Sync;
pub struct Vault {
    storage: Arc<dyn KeychainStorage>,
    account: String,
    owner: String,
    allowed: Mutex<HashSet<String>>,
    ephemeral: bool,
    scanned: AtomicBool,
    detector: Mutex<Option<Arc<Detector>>>,
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
            scanned: AtomicBool::new(false),
            detector: Mutex::new(None),
        }
    }
    pub fn set_detector(&self, detector: Arc<Detector>) {
        *self.detector.lock().expect("detector lock") = Some(detector);
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
        self.storage
            .store(SERVICE, &self.index_key(), &index.to_string())
            .map_err(|_| storage_error())
    }
    pub fn bind_response(&self, id: &str) -> AppResult<()> {
        let _lock = STORE_LOCK.lock().map_err(|_| storage_error())?;
        let entries = self.load()?;
        self.save(&entries)?;
        let mut index = self.index()?;
        index["responses"][id] = Value::String(self.account.clone());
        self.save_index(&index)
    }
    pub fn continue_response(
        storage: Arc<dyn KeychainStorage>,
        owner: &str,
        id: &str,
    ) -> AppResult<Self> {
        let mut vault = Self::new(storage, owner, "", false);
        let _lock = STORE_LOCK.lock().map_err(|_| storage_error())?;
        let index = vault.index()?;
        let account = index["responses"][id]
            .as_str()
            .ok_or_else(error)?
            .to_owned();
        unpin(&vault.account);
        vault.account = account;
        pin(&vault.account);
        if vault
            .storage
            .get(SERVICE, &vault.account)
            .map_err(|_| storage_error())?
            .is_none()
        {
            return Err(error());
        }
        let entries = vault.load()?;
        vault
            .allowed
            .lock()
            .map_err(|_| storage_error())?
            .extend(entries.keys().cloned());
        Ok(vault)
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
        vault.save_index(&index)
    }
    fn load(&self) -> AppResult<BTreeMap<String, Zeroizing<String>>> {
        let index = self.index()?;
        if index["revoked"] == true || index["revoked_scopes"][&self.account] == true {
            return Err(error());
        }
        let Some(raw) = self
            .storage
            .get(SERVICE, &self.account)
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
        unpin(&self.account);
        if self.ephemeral {
            if self.forget().is_err() {
                tracing::warn!("Could not clean up request-local secret credential record");
            }
        }
    }
}
impl SecretContext for Vault {
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
    fn protect(&self, value: &mut Value) -> AppResult<()> {
        let detector = self.detector.lock().map_err(|_| storage_error())?.clone();
        if let Some(detector) = detector {
            for secret in detector(value)? {
                self.register(&secret)?;
            }
        }
        let _lock = STORE_LOCK.lock().map_err(|_| storage_error())?;
        let mut entries = self.load()?;
        let count = entries.len();
        fn signed_secret(value: &Value, entries: &BTreeMap<String, Zeroizing<String>>) -> bool {
            match value {
                Value::Object(map) => {
                    if ["signature", "thought_signature", "encrypted_content"]
                        .iter()
                        .any(|key| {
                            map.get(*key)
                                .and_then(Value::as_str)
                                .is_some_and(|s| !s.is_empty())
                        })
                    {
                        fn contains(
                            value: &Value,
                            entries: &BTreeMap<String, Zeroizing<String>>,
                        ) -> bool {
                            match value {
                                Value::String(s) => {
                                    entries.values().any(|secret| s.contains(secret.as_str()))
                                }
                                Value::Object(m) => m.values().any(|v| contains(v, entries)),
                                Value::Array(a) => a.iter().any(|v| contains(v, entries)),
                                _ => false,
                            }
                        }
                        contains(value, entries)
                    } else {
                        map.values().any(|v| signed_secret(v, entries))
                    }
                }
                Value::Array(a) => a.iter().any(|v| signed_secret(v, entries)),
                _ => false,
            }
        }
        if signed_secret(value, &entries) {
            return Err(AppError::InvalidParams(
                "A signed provider block contains a secret and cannot be safely redacted".into(),
            ));
        }
        visit(
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
                self.transform(text, &mut entries)
            },
            false,
        )?;
        if !entries.is_empty() && (entries.len() != count || self.active()) {
            self.save(&entries)?;
        }
        Ok(())
    }
    fn restore_text(&self, text: &str, strict: bool) -> AppResult<String> {
        if !text.to_ascii_lowercase().contains(TAG) {
            return Ok(text.into());
        }
        let _lock = STORE_LOCK.lock().map_err(|_| storage_error())?;
        let entries = self.load()?;
        let allowed = self.allowed.lock().map_err(|_| storage_error())?;
        restore_text(text, strict, |id| {
            Ok(entries
                .get(id)
                .filter(|_| allowed.contains(id))
                .map(|s| s.to_string()))
        })
    }
    fn restore(&self, value: &mut Value, strict: bool) -> AppResult<()> {
        visit(
            value,
            &mut |text, executable| self.restore_text(text, strict || executable),
            !strict,
        )
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
    match value {
        Value::String(s) => *s = f(s, false)?,
        Value::Array(items) => {
            for item in items {
                visit(item, f, response)?;
            }
        }
        Value::Object(map) => {
            if response
                && ["signature", "thought_signature", "encrypted_content"]
                    .iter()
                    .any(|key| {
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
                    ) || kind.starts_with("response.")
                        || kind.starts_with("content_block")
                })
                || (map.contains_key("name") && map.contains_key("arguments"));
            let mut replaced = serde_json::Map::new();
            for (key, mut item) in std::mem::take(map) {
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
                    visit(
                        &mut item,
                        &mut |s, exec| f(s, executable || exec),
                        response && !executable,
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
    #[test]
    #[ignore = "Explicit native keychain proof; creates and deletes only a synthetic isolated credential"]
    fn native_os_store_round_trip() {
        let session = format!("native-proof-{}", std::process::id());
        let owner = format!("synthetic-test-owner-{}", std::process::id());
        let store = Arc::new(lr_api_keys::keychain_trait::SystemKeychain);
        let vault = Vault::new(store.clone(), &owner, &session, false);
        vault.register("synthetic-native-secret-🔐").unwrap();
        let mut value = serde_json::json!({"content":"synthetic-native-secret-🔐"});
        vault.protect(&mut value).unwrap();
        drop(vault);
        let recovered = Vault::new(store, &owner, &session, false);
        recovered.protect(&mut value).unwrap();
        recovered.restore(&mut value, true).unwrap();
        assert_eq!(value["content"], "synthetic-native-secret-🔐");
        recovered.forget().unwrap();
        recovered
            .storage
            .delete(SERVICE, &recovered.index_key())
            .unwrap();
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
