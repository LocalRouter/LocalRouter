use crate::types::{RepairAction, RepairOptions};
use serde_json::Value;
use std::collections::HashSet;
use tracing::debug;

/// Unified streaming JSON repairer that handles both syntax repair and schema coercion
/// in a single character-at-a-time pass. Minimizes buffering by only buffering tokens
/// (strings, numbers, keywords) and using depth counters for skipping.
pub struct StreamingJsonRepairer {
    // Current parsing state
    phase: Phase,
    token: TokenState,

    // Nesting context with schema info
    stack: Vec<Frame>,

    // Token accumulation buffer (reused across tokens)
    buf: String,
    in_escape: bool,
    quote_char: u8,

    // Trailing comma handling
    comma_pending: bool,
    pending_ws: String, // Whitespace after a pending comma

    // Pre-JSON detection
    pre_buf: String,
    scanning_fence: bool,

    // Schema & options
    schema: Option<Value>,
    options: RepairOptions,

    // Track repair actions
    actions: Vec<RepairAction>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    PreJson,
    Parsing,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum TokenState {
    Between,
    InString,
    InNumber,
    InKeyword,
}

#[derive(Debug, Clone, PartialEq)]
enum ContextType {
    Object,
    Array,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum FramePosition {
    // Object positions
    ExpectKeyOrClose,
    ExpectColon,
    ExpectValue,
    AfterValue,
    // Array positions
    ExpectItemOrClose,
    AfterItem,
}

#[derive(Debug, Clone)]
struct Frame {
    ctx: ContextType,
    position: FramePosition,

    // Schema for this container
    schema: Option<Value>,

    // Object-specific
    current_key: Option<String>,
    value_schema: Option<Value>,
    seen_keys: Option<HashSet<String>>,
    skip_mode: bool,
    skip_depth: u32,
    had_value: bool,
}

impl Frame {
    fn new_object(schema: Option<Value>, track_keys: bool) -> Self {
        Self {
            ctx: ContextType::Object,
            position: FramePosition::ExpectKeyOrClose,
            schema,
            current_key: None,
            value_schema: None,
            seen_keys: if track_keys {
                Some(HashSet::new())
            } else {
                None
            },
            skip_mode: false,
            skip_depth: 0,
            had_value: false,
        }
    }

    fn new_array(schema: Option<Value>) -> Self {
        Self {
            ctx: ContextType::Array,
            position: FramePosition::ExpectItemOrClose,
            schema,
            current_key: None,
            value_schema: None,
            seen_keys: None,
            skip_mode: false,
            skip_depth: 0,
            had_value: false,
        }
    }

    /// Get the schema for array items
    fn items_schema(&self) -> Option<Value> {
        self.schema.as_ref()?.get("items").cloned()
    }

    /// Get the properties map from schema
    fn properties(&self) -> Option<&serde_json::Map<String, Value>> {
        self.schema.as_ref()?.get("properties")?.as_object()
    }

    /// Whether additionalProperties is explicitly false
    fn additional_properties_false(&self) -> bool {
        self.schema
            .as_ref()
            .and_then(|s| s.get("additionalProperties"))
            .and_then(|a| a.as_bool())
            == Some(false)
    }
}

impl StreamingJsonRepairer {
    pub fn new(schema: Option<Value>, options: RepairOptions) -> Self {
        // Always start with PreJson to detect markdown fences and prose
        let phase = Phase::PreJson;

        Self {
            phase,
            token: TokenState::Between,
            stack: Vec::new(),
            buf: String::with_capacity(64),
            in_escape: false,
            quote_char: b'"',
            comma_pending: false,
            pending_ws: String::new(),
            pre_buf: String::new(),
            scanning_fence: false,
            schema,
            options,
            actions: Vec::new(),
        }
    }

    /// Push a content chunk through the repairer.
    /// Returns the repaired output for this chunk.
    pub fn push_content(&mut self, chunk: &str) -> String {
        let mut output = String::with_capacity(chunk.len());
        for c in chunk.chars() {
            self.process_char(c, &mut output);
        }
        output
    }

    /// Alias for push_content (matches plan API name)
    pub fn push(&mut self, chunk: &str) -> String {
        self.push_content(chunk)
    }

    /// Flush remaining buffered content and close open structures.
    pub fn finish(&mut self) -> String {
        let mut output = String::new();

        // Finish any in-progress token
        match self.token {
            TokenState::InString => {
                // Unterminated string - close it
                self.actions.push(RepairAction::SyntaxRepaired);
                self.finish_string(&mut output);
            }
            TokenState::InNumber => {
                self.finish_number(&mut output);
            }
            TokenState::InKeyword => {
                self.finish_keyword(&mut output);
            }
            TokenState::Between => {}
        }

        // Drop any pending comma
        self.comma_pending = false;

        // Close open containers
        while let Some(frame) = self.stack.last() {
            match frame.ctx {
                ContextType::Object => {
                    self.inject_defaults(&mut output);
                    self.stack.pop();
                    output.push('}');
                    self.actions.push(RepairAction::SyntaxRepaired);
                }
                ContextType::Array => {
                    self.stack.pop();
                    output.push(']');
                    self.actions.push(RepairAction::SyntaxRepaired);
                }
            }
        }

        if !output.is_empty() {
            self.phase = Phase::Done;
        }

        output
    }

    /// Get the repair actions accumulated so far.
    pub fn actions(&self) -> &[RepairAction] {
        &self.actions
    }

    /// Take the accumulated repair actions, leaving an empty vec.
    pub fn take_actions(&mut self) -> Vec<RepairAction> {
        std::mem::take(&mut self.actions)
    }

    fn process_char(&mut self, c: char, output: &mut String) {
        match self.phase {
            Phase::PreJson => self.handle_pre_json(c, output),
            Phase::Parsing => self.handle_parsing(c, output),
            Phase::Done => {} // Ignore trailing content
        }
    }

    // ── Pre-JSON detection ──────────────────────────────────────────

    fn handle_pre_json(&mut self, c: char, output: &mut String) {
        if self.scanning_fence {
            // We found ``` - skip until newline (language tag line)
            if c == '\n' {
                self.scanning_fence = false;
                self.pre_buf.clear();
            }
            return;
        }

        self.pre_buf.push(c);

        // Check for markdown fence
        if self.pre_buf.ends_with("```") {
            self.scanning_fence = true;
            self.actions.push(RepairAction::StrippedMarkdownFences);
            return;
        }

        // Check for JSON start
        if c == '{' || c == '[' {
            if self.pre_buf.len() > 1 {
                self.actions.push(RepairAction::StrippedProse);
            }
            self.phase = Phase::Parsing;
            self.pre_buf.clear(); // Free memory
            self.handle_between(c, output);
        }
    }

    // ── Main parsing dispatch ───────────────────────────────────────

    fn handle_parsing(&mut self, c: char, output: &mut String) {
        match self.token {
            TokenState::Between => self.handle_between(c, output),
            TokenState::InString => self.handle_string_char(c, output),
            TokenState::InNumber => self.handle_number_char(c, output),
            TokenState::InKeyword => self.handle_keyword_char(c, output),
        }
    }

    // ── Between tokens ──────────────────────────────────────────────

    /// Check if a missing comma should be auto-injected before a new value/key token.
    fn maybe_inject_comma(&mut self) {
        if let Some(frame) = self.stack.last() {
            let needs_comma = match frame.ctx {
                ContextType::Object => frame.position == FramePosition::AfterValue,
                ContextType::Array => frame.position == FramePosition::AfterItem,
            };
            if needs_comma && !self.comma_pending {
                self.comma_pending = true;
                self.actions.push(RepairAction::SyntaxRepaired);
            }
        }
    }

    fn handle_between(&mut self, c: char, output: &mut String) {
        // In skip mode, track depth but don't emit
        if self.is_skipping() {
            self.handle_skip_char(c);
            return;
        }

        match c {
            // Whitespace - defer if comma is pending (we may need to drop both)
            ' ' | '\t' | '\n' | '\r' => {
                if self.comma_pending {
                    self.pending_ws.push(c);
                } else {
                    output.push(c);
                }
            }

            '{' => {
                self.maybe_inject_comma();
                self.open_object(output);
            }
            '[' => {
                self.maybe_inject_comma();
                self.open_array(output);
            }
            '}' => self.close_object(output),
            ']' => self.close_array(output),

            '"' => {
                self.maybe_inject_comma();
                self.token = TokenState::InString;
                self.quote_char = b'"';
                self.buf.clear();
                self.in_escape = false;
            }
            '\'' => {
                self.maybe_inject_comma();
                // Single quote → double quote
                self.token = TokenState::InString;
                self.quote_char = b'\'';
                self.buf.clear();
                self.in_escape = false;
                self.actions.push(RepairAction::SyntaxRepaired);
            }

            ',' => {
                // Don't emit yet - might be trailing
                self.comma_pending = true;
            }

            ':' => {
                output.push(':');
                if let Some(frame) = self.stack.last_mut() {
                    if frame.position == FramePosition::ExpectColon {
                        frame.position = FramePosition::ExpectValue;
                    }
                }
            }

            // Number start
            '-' | '0'..='9' => {
                self.maybe_inject_comma();
                self.token = TokenState::InNumber;
                self.buf.clear();
                self.buf.push(c);
            }
            '.' => {
                self.maybe_inject_comma();
                // Leading dot: .5 → 0.5
                self.token = TokenState::InNumber;
                self.buf.clear();
                self.buf.push('0');
                self.buf.push('.');
                self.actions.push(RepairAction::SyntaxRepaired);
            }

            // Keyword/unquoted key start
            'a'..='z' | 'A'..='Z' | '_' | '$' => {
                self.maybe_inject_comma();
                self.token = TokenState::InKeyword;
                self.buf.clear();
                self.buf.push(c);
            }

            _ => {
                // Unknown character - skip it (syntax repair)
            }
        }
    }

    // ── Object/Array open/close ─────────────────────────────────────

    fn open_object(&mut self, output: &mut String) {
        self.emit_pending_comma(output);

        // Determine schema for this object
        let obj_schema = self.current_value_schema();

        let track_keys = self.options.add_defaults
            && obj_schema
                .as_ref()
                .and_then(|s| s.get("required"))
                .is_some();

        self.stack.push(Frame::new_object(obj_schema, track_keys));
        output.push('{');
    }

    fn open_array(&mut self, output: &mut String) {
        self.emit_pending_comma(output);

        let arr_schema = self.current_value_schema();
        self.stack.push(Frame::new_array(arr_schema));
        output.push('[');
    }

    fn close_object(&mut self, output: &mut String) {
        // Drop trailing comma and any deferred whitespace
        self.comma_pending = false;
        self.pending_ws.clear();

        // Inject missing defaults before closing
        self.inject_defaults(output);

        output.push('}');
        self.stack.pop();

        // Update parent frame position
        self.mark_value_complete();

        // Check if root is done
        if self.stack.is_empty() {
            self.phase = Phase::Done;
        }
    }

    fn close_array(&mut self, output: &mut String) {
        // Drop trailing comma and any deferred whitespace
        self.comma_pending = false;
        self.pending_ws.clear();

        output.push(']');
        self.stack.pop();

        // Update parent frame position
        self.mark_value_complete();

        // Check if root is done
        if self.stack.is_empty() {
            self.phase = Phase::Done;
        }
    }

    // ── String handling ─────────────────────────────────────────────

    fn handle_string_char(&mut self, c: char, output: &mut String) {
        if self.in_escape {
            self.in_escape = false;
            self.buf.push(c);
            return;
        }

        if c == '\\' {
            self.in_escape = true;
            self.buf.push(c);
            return;
        }

        let quote = self.quote_char;
        if (quote == b'"' && c == '"') || (quote == b'\'' && c == '\'') {
            // String complete
            self.finish_string(output);
            return;
        }

        self.buf.push(c);
    }

    fn finish_string(&mut self, output: &mut String) {
        self.token = TokenState::Between;
        let was_single_quote = self.quote_char == b'\'';
        let mut s = std::mem::take(&mut self.buf);

        // For single-quote strings, we need to escape double quotes in the content
        // and unescape escaped single quotes (since we're converting to double quotes)
        if was_single_quote {
            let mut fixed = String::with_capacity(s.len());
            let mut chars = s.chars();
            while let Some(c) = chars.next() {
                if c == '\\' {
                    if let Some(next) = chars.next() {
                        if next == '\'' {
                            // Escaped single quote in single-quoted string → just a single quote
                            fixed.push('\'');
                        } else {
                            fixed.push('\\');
                            fixed.push(next);
                        }
                    } else {
                        fixed.push('\\');
                    }
                } else if c == '"' {
                    fixed.push_str("\\\"");
                } else {
                    fixed.push(c);
                }
            }
            s = fixed;
        }

        // If we're in skip mode, just track depth (strings don't change depth)
        if self.is_skipping() {
            self.handle_skip_value_complete();
            return;
        }

        // Determine if this is a key or a value based on frame position
        let is_key = self
            .stack
            .last()
            .map(|f| {
                f.ctx == ContextType::Object
                    && (f.position == FramePosition::ExpectKeyOrClose
                        || f.position == FramePosition::AfterValue)
            })
            .unwrap_or(false);

        if is_key {
            self.handle_key(s, output);
        } else {
            self.handle_string_value(s, output);
        }
    }

    fn handle_key(&mut self, key: String, output: &mut String) {
        if self.stack.last().is_none() {
            self.emit_pending_comma(output);
            // No frame - just emit the key
            output.push('"');
            output.push_str(&key);
            output.push('"');
            return;
        }

        // Track seen keys for default injection
        if let Some(ref mut seen) = self.stack.last_mut().unwrap().seen_keys {
            seen.insert(key.clone());
        }

        // Determine value schema and whether to skip - using immutable borrows only
        let (value_schema, should_skip) = {
            let frame = self.stack.last().unwrap();
            if let Some(props) = frame.properties() {
                if props.contains_key(&key) {
                    (props.get(&key).cloned(), false)
                } else {
                    let ap_false = frame.additional_properties_false();
                    (None, self.options.strip_extra_fields && ap_false)
                }
            } else {
                (None, false)
            }
        };

        if should_skip {
            // Drop pending comma and deferred whitespace - we're removing this field entirely
            self.comma_pending = false;
            self.pending_ws.clear();
            let path = self.build_path(&key);
            debug!("Removed extra field: {}", path);
            self.actions.push(RepairAction::ExtraFieldRemoved { path });
            let frame = self.stack.last_mut().unwrap();
            frame.value_schema = None;
            frame.skip_mode = true;
            frame.skip_depth = 0;
            frame.position = FramePosition::ExpectColon;
        } else {
            self.emit_pending_comma(output);
            // Emit the key
            output.push('"');
            output.push_str(&key);
            output.push('"');

            let frame = self.stack.last_mut().unwrap();
            frame.value_schema = value_schema;
            frame.current_key = Some(key);
            frame.position = FramePosition::ExpectColon;
        }
    }

    fn handle_string_value(&mut self, s: String, output: &mut String) {
        self.emit_pending_comma(output);

        let value_schema = self.current_value_schema_ref();
        let path = self.build_current_path();

        // Enum normalization
        let s = if self.options.normalize_enums {
            if let Some(normalized) = self.try_normalize_enum(&s, value_schema.as_ref()) {
                self.actions.push(RepairAction::EnumNormalized {
                    path: path.clone(),
                    from: s.clone(),
                    to: normalized.clone(),
                });
                normalized
            } else {
                s
            }
        } else {
            s
        };

        // Type coercion
        let schema_type = value_schema
            .as_ref()
            .and_then(|s| s.get("type"))
            .and_then(|t| t.as_str())
            .unwrap_or("");

        if self.options.schema_coercion {
            match schema_type {
                "integer" => {
                    let trimmed = s.trim();
                    if let Ok(i) = trimmed.parse::<i64>() {
                        self.actions.push(RepairAction::TypeCoerced {
                            path,
                            from: "string".to_string(),
                            to: "integer".to_string(),
                        });
                        output.push_str(&i.to_string());
                        self.mark_value_complete();
                        return;
                    }
                }
                "number" => {
                    let trimmed = s.trim();
                    if let Ok(f) = trimmed.parse::<f64>() {
                        if f.is_finite() {
                            self.actions.push(RepairAction::TypeCoerced {
                                path,
                                from: "string".to_string(),
                                to: "number".to_string(),
                            });
                            output.push_str(&format_f64(f));
                            self.mark_value_complete();
                            return;
                        }
                    }
                }
                "boolean" => {
                    let lower = s.trim().to_lowercase();
                    let b = match lower.as_str() {
                        "true" | "1" | "yes" | "on" => Some(true),
                        "false" | "0" | "no" | "off" | "" => Some(false),
                        _ => None,
                    };
                    if let Some(b) = b {
                        self.actions.push(RepairAction::TypeCoerced {
                            path,
                            from: "string".to_string(),
                            to: "boolean".to_string(),
                        });
                        output.push_str(if b { "true" } else { "false" });
                        self.mark_value_complete();
                        return;
                    }
                }
                _ => {}
            }
        }

        // Emit as string (buffer content is already JSON-escaped)
        output.push('"');
        output.push_str(&s);
        output.push('"');
        self.mark_value_complete();
    }

    // ── Number handling ─────────────────────────────────────────────

    fn handle_number_char(&mut self, c: char, output: &mut String) {
        if c.is_ascii_digit() || c == '.' || c == 'e' || c == 'E' || c == '+' || c == '-' {
            // '+' and '-' are only valid after 'e'/'E', but we'll fix in finish_number
            if (c == '+' || c == '-') && !self.buf.ends_with('e') && !self.buf.ends_with('E') {
                // Not part of number - finish and reprocess
                self.finish_number(output);
                self.handle_between(c, output);
                return;
            }
            self.buf.push(c);
        } else {
            // Number complete
            self.finish_number(output);
            // Reprocess current char
            self.handle_between(c, output);
        }
    }

    fn finish_number(&mut self, output: &mut String) {
        self.token = TokenState::Between;
        let num_str = std::mem::take(&mut self.buf);

        if self.is_skipping() {
            self.handle_skip_value_complete();
            return;
        }

        self.emit_pending_comma(output);

        // Fix number issues
        let mut fixed = num_str;
        let mut syntax_fixed = false;

        // Trailing dot: "1." → "1.0"
        if fixed.ends_with('.') {
            fixed.push('0');
            syntax_fixed = true;
        }

        // Incomplete exponent: "1e" → "1"
        if fixed.ends_with('e') || fixed.ends_with('E') {
            fixed.pop();
            syntax_fixed = true;
        }

        // Incomplete exponent with sign: "1e+" → "1"
        if fixed.ends_with("e+")
            || fixed.ends_with("e-")
            || fixed.ends_with("E+")
            || fixed.ends_with("E-")
        {
            fixed.pop();
            fixed.pop();
            syntax_fixed = true;
        }

        if syntax_fixed {
            self.actions.push(RepairAction::SyntaxRepaired);
        }

        // Type coercion
        let value_schema = self.current_value_schema_ref();
        let schema_type = value_schema
            .as_ref()
            .and_then(|s| s.get("type"))
            .and_then(|t| t.as_str())
            .unwrap_or("");

        if self.options.schema_coercion {
            match schema_type {
                "string" => {
                    let path = self.build_current_path();
                    self.actions.push(RepairAction::TypeCoerced {
                        path,
                        from: "number".to_string(),
                        to: "string".to_string(),
                    });
                    output.push('"');
                    output.push_str(&fixed);
                    output.push('"');
                    self.mark_value_complete();
                    return;
                }
                "integer" => {
                    if let Ok(f) = fixed.parse::<f64>() {
                        let i = f as i64;
                        if (f - i as f64).abs() > f64::EPSILON {
                            let path = self.build_current_path();
                            self.actions.push(RepairAction::TypeCoerced {
                                path,
                                from: "number".to_string(),
                                to: "integer".to_string(),
                            });
                            output.push_str(&i.to_string());
                            self.mark_value_complete();
                            return;
                        }
                    }
                }
                "boolean" => {
                    if let Ok(f) = fixed.parse::<f64>() {
                        let path = self.build_current_path();
                        self.actions.push(RepairAction::TypeCoerced {
                            path,
                            from: "number".to_string(),
                            to: "boolean".to_string(),
                        });
                        output.push_str(if f != 0.0 { "true" } else { "false" });
                        self.mark_value_complete();
                        return;
                    }
                }
                _ => {}
            }
        }

        output.push_str(&fixed);
        self.mark_value_complete();
    }

    // ── Keyword handling ────────────────────────────────────────────

    fn handle_keyword_char(&mut self, c: char, output: &mut String) {
        if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
            self.buf.push(c);
        } else {
            // Could be an unquoted key if next non-ws char is ':'
            if c == ':' {
                // This was an unquoted key
                self.finish_unquoted_key(output);
                // Process the colon
                self.handle_between(c, output);
                return;
            }
            // Keyword complete
            self.finish_keyword(output);
            // Reprocess current char
            self.handle_between(c, output);
        }
    }

    fn finish_unquoted_key(&mut self, output: &mut String) {
        self.token = TokenState::Between;
        let key = std::mem::take(&mut self.buf);
        self.actions.push(RepairAction::SyntaxRepaired);
        self.handle_key(key, output);
    }

    fn finish_keyword(&mut self, output: &mut String) {
        self.token = TokenState::Between;
        let kw = std::mem::take(&mut self.buf);

        if self.is_skipping() {
            self.handle_skip_value_complete();
            return;
        }

        self.emit_pending_comma(output);

        let lower = kw.to_lowercase();

        // Check if this is an unquoted key (position suggests we expect a key)
        let is_key_position = self
            .stack
            .last()
            .map(|f| {
                f.ctx == ContextType::Object
                    && (f.position == FramePosition::ExpectKeyOrClose
                        || f.position == FramePosition::AfterValue)
            })
            .unwrap_or(false);

        if is_key_position {
            // Treat as unquoted key
            self.actions.push(RepairAction::SyntaxRepaired);
            self.handle_key(kw, output);
            return;
        }

        let value_schema = self.current_value_schema_ref();
        let schema_type = value_schema
            .as_ref()
            .and_then(|s| s.get("type"))
            .and_then(|t| t.as_str())
            .unwrap_or("");

        // Normalize keywords
        let needs_repair = kw != lower
            || matches!(
                lower.as_str(),
                "none" | "undefined" | "nan" | "infinity" | "yes" | "no"
            );

        match lower.as_str() {
            "true" | "yes" => {
                if needs_repair {
                    self.actions.push(RepairAction::SyntaxRepaired);
                }
                if self.options.schema_coercion {
                    match schema_type {
                        "string" => {
                            let path = self.build_current_path();
                            self.actions.push(RepairAction::TypeCoerced {
                                path,
                                from: "boolean".to_string(),
                                to: "string".to_string(),
                            });
                            output.push_str("\"true\"");
                        }
                        "number" | "integer" => {
                            let path = self.build_current_path();
                            self.actions.push(RepairAction::TypeCoerced {
                                path,
                                from: "boolean".to_string(),
                                to: schema_type.to_string(),
                            });
                            output.push('1');
                        }
                        _ => output.push_str("true"),
                    }
                } else {
                    output.push_str("true");
                }
            }
            "false" | "no" => {
                if needs_repair {
                    self.actions.push(RepairAction::SyntaxRepaired);
                }
                if self.options.schema_coercion {
                    match schema_type {
                        "string" => {
                            let path = self.build_current_path();
                            self.actions.push(RepairAction::TypeCoerced {
                                path,
                                from: "boolean".to_string(),
                                to: "string".to_string(),
                            });
                            output.push_str("\"false\"");
                        }
                        "number" | "integer" => {
                            let path = self.build_current_path();
                            self.actions.push(RepairAction::TypeCoerced {
                                path,
                                from: "boolean".to_string(),
                                to: schema_type.to_string(),
                            });
                            output.push('0');
                        }
                        _ => output.push_str("false"),
                    }
                } else {
                    output.push_str("false");
                }
            }
            "null" | "none" | "undefined" => {
                if needs_repair {
                    self.actions.push(RepairAction::SyntaxRepaired);
                }
                if self.options.schema_coercion && schema_type == "string" {
                    let path = self.build_current_path();
                    self.actions.push(RepairAction::TypeCoerced {
                        path,
                        from: "null".to_string(),
                        to: "string".to_string(),
                    });
                    output.push_str("\"\"");
                } else {
                    output.push_str("null");
                }
            }
            "nan" | "infinity" => {
                self.actions.push(RepairAction::SyntaxRepaired);
                output.push_str("null");
            }
            _ => {
                // Unrecognized keyword - treat as unquoted string
                self.actions.push(RepairAction::SyntaxRepaired);
                output.push('"');
                push_escaped(output, &kw);
                output.push('"');
            }
        }
        self.mark_value_complete();
    }

    // ── Skip mode (for extra field removal) ─────────────────────────

    fn is_skipping(&self) -> bool {
        self.stack.last().map(|f| f.skip_mode).unwrap_or(false)
    }

    fn handle_skip_char(&mut self, c: char) {
        let frame = match self.stack.last_mut() {
            Some(f) => f,
            None => return,
        };

        // We're skipping a field's value. Track depth.
        match c {
            ':' if frame.skip_depth == 0 && frame.position == FramePosition::ExpectColon => {
                frame.position = FramePosition::ExpectValue;
            }
            '{' | '[' => {
                frame.skip_depth += 1;
            }
            '}' | ']' => {
                if frame.skip_depth > 0 {
                    frame.skip_depth -= 1;
                } else {
                    // The closing bracket belongs to the parent - end skip mode
                    frame.skip_mode = false;
                    frame.position = FramePosition::ExpectKeyOrClose;
                }
            }
            ':' => {
                if frame.skip_depth == 0 && frame.position == FramePosition::ExpectColon {
                    frame.position = FramePosition::ExpectValue;
                }
            }
            '"' | '\'' => {
                // Start tracking a string within skipped content
                // We need to handle this to properly track string boundaries
                self.token = TokenState::InString;
                self.quote_char = if c == '\'' { b'\'' } else { b'"' };
                self.buf.clear();
                self.in_escape = false;
            }
            ',' => {
                if frame.skip_depth == 0 && frame.position != FramePosition::ExpectColon {
                    // Comma at our level - value is complete, end skip
                    let had_emitted_value = frame.had_value;
                    frame.skip_mode = false;
                    frame.position = FramePosition::ExpectKeyOrClose;
                    // If the object had previously emitted values, we need a comma
                    // before the next non-skipped field
                    self.comma_pending = had_emitted_value;
                    self.pending_ws.clear();
                }
            }
            _ => {
                // Other chars (whitespace, digits, letters) - just skip
            }
        }
    }

    fn handle_skip_value_complete(&mut self) {
        if let Some(frame) = self.stack.last_mut() {
            if frame.skip_mode && frame.skip_depth == 0 {
                // Value complete at depth 0 - end skip mode
                // Position will be updated by the next comma or closer
                frame.skip_mode = false;
                frame.position = FramePosition::AfterValue;
            }
        }
    }

    // ── Helpers ─────────────────────────────────────────────────────

    fn emit_pending_comma(&mut self, output: &mut String) {
        if self.comma_pending {
            self.comma_pending = false;
            output.push(',');
            // Also flush any whitespace that was deferred after the comma
            if !self.pending_ws.is_empty() {
                output.push_str(&self.pending_ws);
                self.pending_ws.clear();
            }
        }
    }

    fn mark_value_complete(&mut self) {
        if let Some(frame) = self.stack.last_mut() {
            frame.had_value = true;
            match frame.ctx {
                ContextType::Object => {
                    frame.position = FramePosition::AfterValue;
                }
                ContextType::Array => {
                    frame.position = FramePosition::AfterItem;
                }
            }
        }
    }

    /// Get the schema for the value currently being parsed.
    fn current_value_schema(&self) -> Option<Value> {
        match self.stack.last() {
            Some(frame) => match frame.ctx {
                ContextType::Object => {
                    // Value schema was set when we processed the key
                    frame.value_schema.clone()
                }
                ContextType::Array => {
                    // Array items schema
                    frame.items_schema()
                }
            },
            None => {
                // Root level - use the top-level schema
                self.schema.clone()
            }
        }
    }

    /// Same as current_value_schema but returns a reference-based clone for checking type.
    fn current_value_schema_ref(&self) -> Option<Value> {
        self.current_value_schema()
    }

    fn try_normalize_enum(&self, s: &str, schema: Option<&Value>) -> Option<String> {
        let schema = schema?;
        let enum_values = schema.get("enum")?.as_array()?;
        let lower = s.to_lowercase();
        for ev in enum_values {
            if let Value::String(es) = ev {
                if es.to_lowercase() == lower && es != s {
                    return Some(es.clone());
                }
            }
        }
        None
    }

    fn build_path(&self, key: &str) -> String {
        let mut parts = Vec::new();
        for frame in &self.stack {
            if let Some(ref k) = frame.current_key {
                parts.push(k.clone());
            }
        }
        parts.push(key.to_string());
        parts.join(".")
    }

    fn build_current_path(&self) -> String {
        let mut parts = Vec::new();
        for (i, frame) in self.stack.iter().enumerate() {
            match frame.ctx {
                ContextType::Object => {
                    if let Some(ref k) = frame.current_key {
                        parts.push(k.clone());
                    }
                }
                ContextType::Array => {
                    // For array items, we'd need an index counter.
                    // Use a placeholder for now.
                    if i > 0 {
                        parts.push(format!("[{}]", "?"));
                    }
                }
            }
        }
        parts.join(".")
    }

    fn inject_defaults(&mut self, output: &mut String) {
        if !self.options.add_defaults {
            return;
        }

        // Extract everything we need from the frame before the loop
        let (schema, seen_keys, had_value) = match self.stack.last() {
            Some(f) if f.ctx == ContextType::Object => {
                let schema = match &f.schema {
                    Some(s) => s.clone(),
                    None => return,
                };
                let seen = match &f.seen_keys {
                    Some(s) => s.clone(),
                    None => return,
                };
                (schema, seen, f.had_value)
            }
            _ => return,
        };

        let required = match schema.get("required").and_then(|r| r.as_array()) {
            Some(r) => r.clone(),
            None => return,
        };

        let properties = match schema.get("properties").and_then(|p| p.as_object()) {
            Some(p) => p.clone(),
            None => return,
        };

        let path_prefix = self.build_current_path();
        let mut emitted_any = false;

        for req in &required {
            if let Some(req_key) = req.as_str() {
                if !seen_keys.contains(req_key) {
                    if let Some(prop_schema) = properties.get(req_key) {
                        if let Some(default_val) = prop_schema.get("default") {
                            let field_path = if path_prefix.is_empty() {
                                req_key.to_string()
                            } else {
                                format!("{}.{}", path_prefix, req_key)
                            };

                            // Emit comma if needed
                            if had_value || emitted_any {
                                output.push(',');
                            }

                            output.push('"');
                            push_escaped(output, req_key);
                            output.push_str("\":");
                            output
                                .push_str(&serde_json::to_string(default_val).unwrap_or_default());

                            self.actions
                                .push(RepairAction::DefaultAdded { path: field_path });
                            emitted_any = true;
                        }
                    }
                }
            }
        }

        if emitted_any {
            if let Some(frame) = self.stack.last_mut() {
                frame.had_value = true;
            }
        }
    }
}

/// Push a string with JSON escaping (only escape what's necessary).
fn push_escaped(output: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                output.push_str(&format!("\\u{:04x}", c as u32));
            }
            _ => output.push(c),
        }
    }
}

/// Format f64 without unnecessary trailing zeros
fn format_f64(f: f64) -> String {
    if f == f.trunc() && f.abs() < 1e15 {
        // It's an integer value - format without decimal
        format!("{}", f as i64)
    } else {
        format!("{}", f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── Syntax repair tests ─────────────────────────────────────────

    #[test]
    fn test_valid_json_unchanged() {
        let mut r = StreamingJsonRepairer::new(None, RepairOptions::default());
        let mut out = r.push(r#"{"name": "John", "age": 30}"#);
        out.push_str(&r.finish());
        assert_eq!(
            serde_json::from_str::<Value>(&out).unwrap(),
            json!({"name": "John", "age": 30})
        );
    }

    #[test]
    fn test_trailing_comma_object() {
        let mut r = StreamingJsonRepairer::new(None, RepairOptions::default());
        let mut out = r.push(r#"{"name": "John", "age": 30,}"#);
        out.push_str(&r.finish());
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v, json!({"name": "John", "age": 30}));
    }

    #[test]
    fn test_trailing_comma_array() {
        let mut r = StreamingJsonRepairer::new(None, RepairOptions::default());
        let mut out = r.push("[1, 2, 3,]");
        out.push_str(&r.finish());
        assert_eq!(
            serde_json::from_str::<Value>(&out).unwrap(),
            json!([1, 2, 3])
        );
    }

    #[test]
    fn test_markdown_fences_json() {
        let mut r = StreamingJsonRepairer::new(None, RepairOptions::default());
        let mut out = r.push("```json\n{\"name\": \"John\"}\n```");
        out.push_str(&r.finish());
        assert_eq!(
            serde_json::from_str::<Value>(&out).unwrap(),
            json!({"name": "John"})
        );
    }

    #[test]
    fn test_markdown_fences_no_lang() {
        let mut r = StreamingJsonRepairer::new(None, RepairOptions::default());
        let mut out = r.push("```\n{\"key\": \"value\"}\n```");
        out.push_str(&r.finish());
        assert!(serde_json::from_str::<Value>(&out).is_ok());
    }

    #[test]
    fn test_prose_around_json() {
        let mut r = StreamingJsonRepairer::new(None, RepairOptions::default());
        let mut out = r.push("Here is the JSON:\n{\"name\": \"John\"}");
        out.push_str(&r.finish());
        assert!(serde_json::from_str::<Value>(&out).is_ok());
    }

    #[test]
    fn test_missing_closing_bracket() {
        let mut r = StreamingJsonRepairer::new(None, RepairOptions::default());
        let mut out = r.push(r#"{"name": "John", "age": 30"#);
        out.push_str(&r.finish());
        assert!(
            serde_json::from_str::<Value>(&out).is_ok(),
            "Failed: {}",
            out
        );
    }

    #[test]
    fn test_single_quotes() {
        let mut r = StreamingJsonRepairer::new(None, RepairOptions::default());
        let mut out = r.push("{'name': 'John'}");
        out.push_str(&r.finish());
        assert!(
            serde_json::from_str::<Value>(&out).is_ok(),
            "Failed: {}",
            out
        );
    }

    #[test]
    fn test_unquoted_keys() {
        let mut r = StreamingJsonRepairer::new(None, RepairOptions::default());
        let mut out = r.push("{name: \"John\"}");
        out.push_str(&r.finish());
        assert!(
            serde_json::from_str::<Value>(&out).is_ok(),
            "Failed: {}",
            out
        );
    }

    #[test]
    fn test_markdown_with_trailing_comma() {
        let mut r = StreamingJsonRepairer::new(None, RepairOptions::default());
        let mut out = r.push("```json\n{\"name\": \"John\", \"age\": 30,}\n```");
        out.push_str(&r.finish());
        assert!(serde_json::from_str::<Value>(&out).is_ok());
    }

    #[test]
    fn test_empty_object() {
        let mut r = StreamingJsonRepairer::new(None, RepairOptions::default());
        let mut out = r.push("{}");
        out.push_str(&r.finish());
        assert_eq!(serde_json::from_str::<Value>(&out).unwrap(), json!({}));
    }

    #[test]
    fn test_empty_array() {
        let mut r = StreamingJsonRepairer::new(None, RepairOptions::default());
        let mut out = r.push("[]");
        out.push_str(&r.finish());
        assert_eq!(serde_json::from_str::<Value>(&out).unwrap(), json!([]));
    }

    #[test]
    fn test_nested_objects() {
        let mut r = StreamingJsonRepairer::new(None, RepairOptions::default());
        let mut out = r.push(r#"{"user": {"name": "John", "address": {"city": "NYC",},},}"#);
        out.push_str(&r.finish());
        assert!(
            serde_json::from_str::<Value>(&out).is_ok(),
            "Failed: {}",
            out
        );
    }

    #[test]
    fn test_python_keywords() {
        let mut r = StreamingJsonRepairer::new(None, RepairOptions::default());
        let mut out = r.push(r#"{"a": True, "b": False, "c": None}"#);
        out.push_str(&r.finish());
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v, json!({"a": true, "b": false, "c": null}));
    }

    #[test]
    fn test_missing_comma_object() {
        let mut r = StreamingJsonRepairer::new(None, RepairOptions::default());
        let mut out = r.push(r#"{"name": "Alice" "age": 30}"#);
        out.push_str(&r.finish());
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v, json!({"name": "Alice", "age": 30}));
    }

    #[test]
    fn test_missing_comma_array() {
        let mut r = StreamingJsonRepairer::new(None, RepairOptions::default());
        let mut out = r.push("[1 2 3]");
        out.push_str(&r.finish());
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v, json!([1, 2, 3]));
    }

    // ── Streaming tests ─────────────────────────────────────────────

    #[test]
    fn test_streaming_basic() {
        let mut r = StreamingJsonRepairer::new(None, RepairOptions::default());
        let mut result = String::new();
        result.push_str(&r.push(r#"{"name": "#));
        result.push_str(&r.push(r#""John"}"#));
        result.push_str(&r.finish());
        assert!(serde_json::from_str::<Value>(&result).is_ok());
    }

    #[test]
    fn test_streaming_trailing_comma() {
        let mut r = StreamingJsonRepairer::new(None, RepairOptions::default());
        let mut result = String::new();
        result.push_str(&r.push(r#"{"name": "John","#));
        result.push_str(&r.push(r#"}"#));
        result.push_str(&r.finish());
        assert!(
            serde_json::from_str::<Value>(&result).is_ok(),
            "Failed: {}",
            result
        );
    }

    #[test]
    fn test_streaming_missing_closing() {
        let mut r = StreamingJsonRepairer::new(None, RepairOptions::default());
        let mut result = String::new();
        result.push_str(&r.push(r#"{"name": "John""#));
        result.push_str(&r.finish());
        assert!(
            serde_json::from_str::<Value>(&result).is_ok(),
            "Failed: {}",
            result
        );
    }

    #[test]
    fn test_streaming_character_by_character() {
        let input = r#"{"key": "value"}"#;
        let mut r = StreamingJsonRepairer::new(None, RepairOptions::default());
        let mut result = String::new();
        for c in input.chars() {
            result.push_str(&r.push(&c.to_string()));
        }
        result.push_str(&r.finish());
        assert!(
            serde_json::from_str::<Value>(&result).is_ok(),
            "Failed: {}",
            result
        );
    }

    #[test]
    fn test_streaming_multiple_chunks() {
        let mut r = StreamingJsonRepairer::new(None, RepairOptions::default());
        let mut result = String::new();
        result.push_str(&r.push(r#"{"items": "#));
        result.push_str(&r.push(r#"[1, 2"#));
        result.push_str(&r.push(r#", 3,]"#));
        result.push_str(&r.push(r#"}"#));
        result.push_str(&r.finish());
        assert!(
            serde_json::from_str::<Value>(&result).is_ok(),
            "Failed: {}",
            result
        );
    }

    // ── Schema coercion tests ───────────────────────────────────────

    #[test]
    fn test_string_to_number() {
        let schema = json!({"type": "object", "properties": {"val": {"type": "number"}}});
        let mut r = StreamingJsonRepairer::new(
            Some(schema),
            RepairOptions {
                schema_coercion: true,
                ..Default::default()
            },
        );
        let mut out = r.push(r#"{"val": "42.5"}"#);
        out.push_str(&r.finish());
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["val"], json!(42.5));
        assert!(r
            .actions()
            .iter()
            .any(|a| matches!(a, RepairAction::TypeCoerced { .. })));
    }

    #[test]
    fn test_string_to_integer() {
        let schema = json!({"type": "object", "properties": {"val": {"type": "integer"}}});
        let mut r = StreamingJsonRepairer::new(
            Some(schema),
            RepairOptions {
                schema_coercion: true,
                ..Default::default()
            },
        );
        let mut out = r.push(r#"{"val": "42"}"#);
        out.push_str(&r.finish());
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["val"], json!(42));
    }

    #[test]
    fn test_number_to_string() {
        let schema = json!({"type": "object", "properties": {"val": {"type": "string"}}});
        let mut r = StreamingJsonRepairer::new(
            Some(schema),
            RepairOptions {
                schema_coercion: true,
                ..Default::default()
            },
        );
        let mut out = r.push(r#"{"val": 42}"#);
        out.push_str(&r.finish());
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["val"], json!("42"));
    }

    #[test]
    fn test_string_to_boolean() {
        let schema = json!({"type": "object", "properties": {"val": {"type": "boolean"}}});
        let mut r = StreamingJsonRepairer::new(
            Some(schema),
            RepairOptions {
                schema_coercion: true,
                ..Default::default()
            },
        );
        let mut out = r.push(r#"{"val": "true"}"#);
        out.push_str(&r.finish());
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["val"], json!(true));
    }

    #[test]
    fn test_boolean_to_string() {
        let schema = json!({"type": "object", "properties": {"val": {"type": "string"}}});
        let mut r = StreamingJsonRepairer::new(
            Some(schema),
            RepairOptions {
                schema_coercion: true,
                ..Default::default()
            },
        );
        let mut out = r.push(r#"{"val": true}"#);
        out.push_str(&r.finish());
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["val"], json!("true"));
    }

    #[test]
    fn test_strip_extra_fields() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer"}
            },
            "additionalProperties": false
        });
        let opts = RepairOptions {
            strip_extra_fields: true,
            ..Default::default()
        };
        let mut r = StreamingJsonRepairer::new(Some(schema), opts);
        let mut out = r.push(r#"{"name": "John", "age": 30, "extra": "field"}"#);
        out.push_str(&r.finish());
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v, json!({"name": "John", "age": 30}));
        assert!(r
            .actions()
            .iter()
            .any(|a| matches!(a, RepairAction::ExtraFieldRemoved { .. })));
    }

    #[test]
    fn test_add_defaults() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "status": {"type": "string", "default": "active"}
            },
            "required": ["name", "status"]
        });
        let opts = RepairOptions {
            add_defaults: true,
            ..Default::default()
        };
        let mut r = StreamingJsonRepairer::new(Some(schema), opts);
        let mut out = r.push(r#"{"name": "John"}"#);
        out.push_str(&r.finish());
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v, json!({"name": "John", "status": "active"}));
        assert!(r
            .actions()
            .iter()
            .any(|a| matches!(a, RepairAction::DefaultAdded { .. })));
    }

    #[test]
    fn test_enum_normalization() {
        let schema = json!({
            "type": "object",
            "properties": {
                "status": {"type": "string", "enum": ["Active", "Inactive", "Pending"]}
            }
        });
        let opts = RepairOptions {
            normalize_enums: true,
            ..Default::default()
        };
        let mut r = StreamingJsonRepairer::new(Some(schema), opts);
        let mut out = r.push(r#"{"status": "active"}"#);
        out.push_str(&r.finish());
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["status"], json!("Active"));
    }

    #[test]
    fn test_nested_coercion() {
        let schema = json!({
            "type": "object",
            "properties": {
                "user": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"},
                        "age": {"type": "integer"}
                    }
                }
            }
        });
        let opts = RepairOptions {
            schema_coercion: true,
            ..Default::default()
        };
        let mut r = StreamingJsonRepairer::new(Some(schema), opts);
        let mut out = r.push(r#"{"user": {"name": "John", "age": "30"}}"#);
        out.push_str(&r.finish());
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v, json!({"user": {"name": "John", "age": 30}}));
    }

    #[test]
    fn test_array_coercion() {
        let schema = json!({
            "type": "object",
            "properties": {
                "nums": {"type": "array", "items": {"type": "integer"}}
            }
        });
        let opts = RepairOptions {
            schema_coercion: true,
            ..Default::default()
        };
        let mut r = StreamingJsonRepairer::new(Some(schema), opts);
        let mut out = r.push(r#"{"nums": ["1", "2", "3"]}"#);
        out.push_str(&r.finish());
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["nums"], json!([1, 2, 3]));
    }

    #[test]
    fn test_no_coercion_when_valid() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer"}
            }
        });
        let opts = RepairOptions {
            schema_coercion: true,
            ..Default::default()
        };
        let mut r = StreamingJsonRepairer::new(Some(schema), opts);
        let mut out = r.push(r#"{"name": "John", "age": 30}"#);
        out.push_str(&r.finish());
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v, json!({"name": "John", "age": 30}));
    }

    #[test]
    fn test_float_to_integer_truncation() {
        let schema = json!({"type": "object", "properties": {"val": {"type": "integer"}}});
        let opts = RepairOptions {
            schema_coercion: true,
            ..Default::default()
        };
        let mut r = StreamingJsonRepairer::new(Some(schema), opts);
        let mut out = r.push(r#"{"val": 3.7}"#);
        out.push_str(&r.finish());
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["val"], json!(3));
    }

    #[test]
    fn test_combined_operations() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer"},
                "status": {"type": "string", "enum": ["Active", "Inactive"], "default": "Active"}
            },
            "required": ["name", "age", "status"],
            "additionalProperties": false
        });
        let opts = RepairOptions {
            syntax_repair: true,
            schema_coercion: true,
            strip_extra_fields: true,
            add_defaults: true,
            normalize_enums: true,
        };
        let mut r = StreamingJsonRepairer::new(Some(schema), opts);
        let mut out = r.push(r#"{"name": "John", "age": "30", "extra": true}"#);
        out.push_str(&r.finish());
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v, json!({"name": "John", "age": 30, "status": "Active"}));
    }

    // ── Integration: syntax + schema ────────────────────────────────

    #[test]
    fn test_full_pipeline_markdown_coerce() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer"},
                "status": {"type": "string", "enum": ["Active", "Inactive"]}
            },
            "additionalProperties": false
        });
        let opts = RepairOptions {
            syntax_repair: true,
            schema_coercion: true,
            strip_extra_fields: true,
            normalize_enums: true,
            ..Default::default()
        };
        let mut r = StreamingJsonRepairer::new(Some(schema), opts);
        let input = "Here is your data:\n{\"name\": \"John\", \"age\": \"30\", \"extra\": true, \"status\": \"active\",}\nDone!";
        let mut out = r.push(input);
        out.push_str(&r.finish());
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["age"], json!(30));
        assert_eq!(v["status"], json!("Active"));
        assert!(v.get("extra").is_none());
    }

    #[test]
    fn test_disabled_syntax_repair() {
        // Even with syntax_repair disabled, we still parse JSON
        let mut r = StreamingJsonRepairer::new(
            None,
            RepairOptions {
                syntax_repair: false,
                ..Default::default()
            },
        );
        let mut out = r.push(r#"{"name": "John"}"#);
        out.push_str(&r.finish());
        assert!(serde_json::from_str::<Value>(&out).is_ok());
    }

    #[test]
    fn test_null_coercion_to_string() {
        let schema = json!({"type": "object", "properties": {"val": {"type": "string"}}});
        let opts = RepairOptions {
            schema_coercion: true,
            ..Default::default()
        };
        let mut r = StreamingJsonRepairer::new(Some(schema), opts);
        let mut out = r.push(r#"{"val": null}"#);
        out.push_str(&r.finish());
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["val"], json!(""));
    }

    #[test]
    fn test_bool_to_number() {
        let schema = json!({"type": "object", "properties": {"val": {"type": "integer"}}});
        let opts = RepairOptions {
            schema_coercion: true,
            ..Default::default()
        };
        let mut r = StreamingJsonRepairer::new(Some(schema), opts);
        let mut out = r.push(r#"{"val": true}"#);
        out.push_str(&r.finish());
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["val"], json!(1));
    }
}
