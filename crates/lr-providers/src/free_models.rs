//! Which models of a provider count as free for free-tier-only strategies.
//!
//! For a `FreeModelsOnly` provider a model is decided by, in order: the
//! "never free" list, the "always free" list, the first enabled rule that
//! matches, else not free. Rules look at the model id, at the model's own
//! entry in the provider's listing (JSON Pointer fields, JSONPath filters)
//! and at its price (the per-model price override or the listed price).
//!
//! Evaluation fails closed: data that is not known (no listing entry, no
//! price) never makes a model free, and makes an exclude rule apply. A rule
//! that does not compile is reported and treated the same way. Catalog
//! prices (models.dev) are never used: `0` there means "unknown".

use lr_config::{
    FieldOp, FreeModelCondition, FreeModelRule, FreeModelRuleEffect, FreeTierKind,
    ModelIdPatternSyntax, ModelPricingOverride, PriceSource,
};
use regex::{Regex, RegexBuilder};
use serde::Serialize;
use serde_json::Value;
use serde_json_path::JsonPath;

use crate::listing_metadata::{listed_price_fields, price_value};

/// Longest accepted model id pattern, regular expression or JSON Pointer.
pub const MAX_PATTERN_LEN: usize = 1_000;
/// Longest accepted JSONPath expression.
pub const MAX_JSON_PATH_LEN: usize = 1_000;
/// Compiled size limit for one regular expression.
const REGEX_SIZE_LIMIT: usize = 1 << 20;

/// What decided whether a model is free.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecidedBy {
    /// The provider has no free tier
    NoFreeTier,
    /// The provider's free tier covers every model (local, subscription,
    /// rate-limited, credits)
    Provider,
    /// A price set for the model makes it paid on an always-free provider
    PriceOverride,
    /// The model is on the "never free" list
    ExcludedList,
    /// The model is on the "always free" list
    IncludedList,
    /// A rule matched (`rule_index`)
    Rule,
    /// No list entry or rule matched
    NoRule,
}

/// Whether one model is free, and why.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FreeModelDecision {
    pub is_free: bool,
    pub decided_by: DecidedBy,
    /// The deciding rule, when `decided_by` is `Rule`
    pub rule_index: Option<usize>,
    /// Human-readable explanation
    pub reason: String,
}

/// What is known about a model.
#[derive(Debug, Clone, Copy, Default)]
pub struct FreeModelInput<'a> {
    pub model_id: &'a str,
    /// The model's entry in the provider's listing, when the provider keeps
    /// it and the model is listed
    pub metadata: Option<&'a Value>,
    /// The model's price override (this instance's, else the legacy one for
    /// its provider type)
    pub price_override: Option<&'a ModelPricingOverride>,
}

/// A problem with a rule or list entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuleError {
    /// The rule, or `None` for the "always free"/"never free" lists
    pub rule_index: Option<usize>,
    /// The input the problem is in (`pattern`, `pointer`, `value`,
    /// `expression`, `input_pointer`, `output_pointer`, `sources`, `type`,
    /// `effect`, `free_model_patterns`, `excluded_models`)
    pub field: String,
    pub message: String,
}

/// Result of one condition for one model.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Match(String),
    NoMatch(String),
    /// The data needed is not known (fails closed)
    Unknown(String),
}

impl Outcome {
    pub fn is_match(&self) -> bool {
        matches!(self, Outcome::Match(_))
    }
}

/// A `FreeTierKind`, compiled for evaluating many models.
pub struct FreeTierEvaluator {
    kind: EvaluatorKind,
    errors: Vec<RuleError>,
}

enum EvaluatorKind {
    NoFreeTier,
    /// Every model is free unless priced on the Models tab
    AlwaysFree(&'static str),
    /// Every model is free within the provider's limits
    ProviderLimits(&'static str),
    FreeModels {
        excluded: Vec<String>,
        included: Vec<String>,
        rules: Vec<CompiledRule>,
    },
}

struct CompiledRule {
    enabled: bool,
    /// Matches make models free (else not free)
    include: bool,
    condition: Condition,
}

enum Condition {
    ModelId(IdMatcher),
    Field {
        pointer: String,
        op: FieldOp,
        value: Value,
        regex: Option<Regex>,
    },
    JsonPath(Box<JsonPath>),
    ZeroPrice {
        sources: Vec<PriceSource>,
        input_pointer: Option<String>,
        output_pointer: Option<String>,
    },
    /// Did not compile (see the error list)
    Invalid,
}

enum IdMatcher {
    Exact { pattern: String, ci: bool },
    Glob { pattern: String, ci: bool },
    Regex(Regex),
}

impl FreeTierEvaluator {
    pub fn new(kind: &FreeTierKind) -> Self {
        let mut errors = Vec::new();
        let kind = match kind {
            FreeTierKind::None => EvaluatorKind::NoFreeTier,
            FreeTierKind::AlwaysFreeLocal => {
                EvaluatorKind::AlwaysFree("Local provider: every model is free")
            }
            FreeTierKind::Subscription => EvaluatorKind::AlwaysFree("Included in the subscription"),
            FreeTierKind::RateLimitedFree { .. } => {
                EvaluatorKind::ProviderLimits("Free within the provider's rate limits")
            }
            FreeTierKind::CreditBased { .. } => {
                EvaluatorKind::ProviderLimits("Free while the free credits last")
            }
            FreeTierKind::FreeModelsOnly {
                free_model_patterns,
                excluded_models,
                rules,
                ..
            } => EvaluatorKind::FreeModels {
                excluded: compile_list(excluded_models, "excluded_models", &mut errors),
                included: compile_list(free_model_patterns, "free_model_patterns", &mut errors),
                rules: rules
                    .iter()
                    .enumerate()
                    .map(|(i, r)| compile_rule(i, r, &mut errors))
                    .collect(),
            },
        };
        Self { kind, errors }
    }

    /// Problems found while compiling (empty when every rule is valid).
    pub fn errors(&self) -> &[RuleError] {
        &self.errors
    }

    /// Whether an enabled rule reads the model's listing entry, so the
    /// listing should be fetched before deciding.
    pub fn needs_metadata(&self) -> bool {
        let EvaluatorKind::FreeModels { rules, .. } = &self.kind else {
            return false;
        };
        rules
            .iter()
            .filter(|r| r.enabled)
            .any(|r| match &r.condition {
                Condition::Field { .. } | Condition::JsonPath(_) => true,
                Condition::ZeroPrice { sources, .. } => {
                    sources.contains(&PriceSource::ProviderListing)
                }
                Condition::ModelId(_) | Condition::Invalid => false,
            })
    }

    /// Number of rules (compiled or not).
    pub fn rule_count(&self) -> usize {
        match &self.kind {
            EvaluatorKind::FreeModels { rules, .. } => rules.len(),
            _ => 0,
        }
    }

    /// Whether rule `index` matches the model, regardless of order or of
    /// it being enabled (for match counts in the UI).
    pub fn rule_matches(&self, index: usize, input: &FreeModelInput) -> bool {
        match &self.kind {
            EvaluatorKind::FreeModels { rules, .. } => rules
                .get(index)
                .is_some_and(|r| r.condition.eval(input).is_match()),
            _ => false,
        }
    }

    /// Whether the model is free, and why.
    pub fn decide(&self, input: &FreeModelInput) -> FreeModelDecision {
        let decision = |is_free, decided_by, rule_index, reason: String| FreeModelDecision {
            is_free,
            decided_by,
            rule_index,
            reason,
        };
        match &self.kind {
            EvaluatorKind::NoFreeTier => decision(
                false,
                DecidedBy::NoFreeTier,
                None,
                "This provider has no free tier".into(),
            ),
            EvaluatorKind::AlwaysFree(reason) => match input.price_override {
                Some(p) if p.input_per_million > 0.0 || p.output_per_million > 0.0 => decision(
                    false,
                    DecidedBy::PriceOverride,
                    None,
                    format!(
                        "Priced on the Models tab ({} in / {} out per 1M tokens)",
                        usd(p.input_per_million),
                        usd(p.output_per_million)
                    ),
                ),
                _ => decision(true, DecidedBy::Provider, None, (*reason).into()),
            },
            EvaluatorKind::ProviderLimits(reason) => {
                decision(true, DecidedBy::Provider, None, (*reason).into())
            }
            EvaluatorKind::FreeModels {
                excluded,
                included,
                rules,
            } => {
                if let Some(p) = excluded
                    .iter()
                    .find(|p| glob_match(p, input.model_id, false))
                {
                    return decision(
                        false,
                        DecidedBy::ExcludedList,
                        None,
                        format!("Never free (matches \"{p}\")"),
                    );
                }
                if let Some(p) = included
                    .iter()
                    .find(|p| glob_match(p, input.model_id, false))
                {
                    return decision(
                        true,
                        DecidedBy::IncludedList,
                        None,
                        format!("Always free (matches \"{p}\")"),
                    );
                }
                for (i, rule) in rules.iter().enumerate().filter(|(_, r)| r.enabled) {
                    let n = i + 1;
                    match (rule.include, rule.condition.eval(input)) {
                        (true, Outcome::Match(why)) => {
                            return decision(
                                true,
                                DecidedBy::Rule,
                                Some(i),
                                format!("Rule {n}: {why}"),
                            )
                        }
                        (false, Outcome::Match(why)) => {
                            return decision(
                                false,
                                DecidedBy::Rule,
                                Some(i),
                                format!("Rule {n} (exclude): {why}"),
                            )
                        }
                        (false, Outcome::Unknown(why)) => {
                            return decision(
                                false,
                                DecidedBy::Rule,
                                Some(i),
                                format!("Rule {n} (exclude) applies: {why}"),
                            )
                        }
                        _ => {}
                    }
                }
                decision(
                    false,
                    DecidedBy::NoRule,
                    None,
                    if rules.iter().any(|r| r.enabled) {
                        "No rule matches".into()
                    } else {
                        "Not on the free list".into()
                    },
                )
            }
        }
    }
}

/// Validate a free tier (for saving): the compile errors, if any.
pub fn validate(kind: &FreeTierKind) -> Vec<RuleError> {
    FreeTierEvaluator::new(kind).errors
}

fn err(rule_index: Option<usize>, field: &str, message: impl Into<String>) -> RuleError {
    RuleError {
        rule_index,
        field: field.into(),
        message: message.into(),
    }
}

fn compile_list(patterns: &[String], field: &str, errors: &mut Vec<RuleError>) -> Vec<String> {
    let mut out = Vec::new();
    for p in patterns {
        let p = p.trim();
        if p.is_empty() {
            continue;
        }
        if p.len() > MAX_PATTERN_LEN {
            errors.push(err(
                None,
                field,
                format!("A pattern is longer than {MAX_PATTERN_LEN} characters"),
            ));
            continue;
        }
        out.push(p.to_string());
    }
    out
}

fn compile_rule(index: usize, rule: &FreeModelRule, errors: &mut Vec<RuleError>) -> CompiledRule {
    let at = Some(index);
    let include = match rule.effect {
        FreeModelRuleEffect::Include => true,
        FreeModelRuleEffect::Exclude => false,
        FreeModelRuleEffect::Unknown => {
            errors.push(err(at, "effect", "This rule's effect is not supported"));
            false
        }
    };
    let condition = match compile_condition(&rule.when) {
        Ok(c) => c,
        Err((field, message)) => {
            errors.push(err(at, field, message));
            Condition::Invalid
        }
    };
    CompiledRule {
        enabled: rule.enabled,
        include,
        condition,
    }
}

fn compile_regex(pattern: &str, case_insensitive: bool) -> Result<Regex, String> {
    if pattern.len() > MAX_PATTERN_LEN {
        return Err(format!(
            "The expression is longer than {MAX_PATTERN_LEN} characters"
        ));
    }
    RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .size_limit(REGEX_SIZE_LIMIT)
        .dfa_size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|e| match e {
            regex::Error::CompiledTooBig(_) => "The expression is too complex".to_string(),
            // Syntax errors draw a caret diagram over several lines; the
            // last line says what is wrong.
            other => {
                let text = other.to_string();
                let last = text.lines().last().unwrap_or("").trim();
                format!(
                    "Invalid regular expression: {}",
                    last.strip_prefix("error: ").unwrap_or(last)
                )
            }
        })
}

fn check_pointer(pointer: &str) -> Result<(), String> {
    if pointer.len() > MAX_PATTERN_LEN {
        return Err(format!(
            "The path is longer than {MAX_PATTERN_LEN} characters"
        ));
    }
    if !pointer.is_empty() && !pointer.starts_with('/') {
        return Err("A JSON Pointer starts with \"/\" (e.g. /tier)".into());
    }
    Ok(())
}

fn compile_condition(when: &FreeModelCondition) -> Result<Condition, (&'static str, String)> {
    match when {
        FreeModelCondition::ModelId {
            pattern,
            syntax,
            case_insensitive,
        } => {
            let pattern = pattern.trim();
            if pattern.is_empty() {
                return Err(("pattern", "Enter a pattern".into()));
            }
            if pattern.len() > MAX_PATTERN_LEN {
                return Err((
                    "pattern",
                    format!("The pattern is longer than {MAX_PATTERN_LEN} characters"),
                ));
            }
            let ci = *case_insensitive;
            let matcher = match syntax {
                ModelIdPatternSyntax::Exact => IdMatcher::Exact {
                    pattern: pattern.into(),
                    ci,
                },
                ModelIdPatternSyntax::Glob => IdMatcher::Glob {
                    pattern: pattern.into(),
                    ci,
                },
                ModelIdPatternSyntax::Regex => {
                    IdMatcher::Regex(compile_regex(pattern, ci).map_err(|e| ("pattern", e))?)
                }
                ModelIdPatternSyntax::Unknown => {
                    return Err(("pattern", "This pattern syntax is not supported".into()))
                }
            };
            Ok(Condition::ModelId(matcher))
        }
        FreeModelCondition::Field { pointer, op, value } => {
            check_pointer(pointer).map_err(|e| ("pointer", e))?;
            let mut regex = None;
            match op {
                FieldOp::Exists | FieldOp::NotExists => {}
                FieldOp::Equals | FieldOp::NotEquals | FieldOp::Contains => {
                    if value.is_null() || value.as_str() == Some("") {
                        return Err(("value", "Enter a value".into()));
                    }
                }
                FieldOp::Matches => {
                    let Some(pattern) = value.as_str().filter(|s| !s.is_empty()) else {
                        return Err(("value", "Enter a regular expression".into()));
                    };
                    regex = Some(compile_regex(pattern, false).map_err(|e| ("value", e))?);
                }
                FieldOp::LessThan
                | FieldOp::LessOrEqual
                | FieldOp::GreaterThan
                | FieldOp::GreaterOrEqual => {
                    if as_number(value).is_none() {
                        return Err(("value", "Enter a number".into()));
                    }
                }
                FieldOp::Unknown => return Err(("op", "This comparison is not supported".into())),
            }
            Ok(Condition::Field {
                pointer: pointer.clone(),
                op: *op,
                value: value.clone(),
                regex,
            })
        }
        FreeModelCondition::JsonPath { expression } => {
            let expr = expression.trim();
            let expr = expr.strip_prefix('?').unwrap_or(expr).trim();
            if expr.is_empty() {
                return Err(("expression", "Enter a filter expression".into()));
            }
            if expr.len() > MAX_JSON_PATH_LEN {
                return Err((
                    "expression",
                    format!("The expression is longer than {MAX_JSON_PATH_LEN} characters"),
                ));
            }
            let path = JsonPath::parse(&format!("$[?{expr}]"))
                .map_err(|e| ("expression", format!("Invalid JSONPath filter: {e}")))?;
            Ok(Condition::JsonPath(Box::new(path)))
        }
        FreeModelCondition::ZeroPrice {
            sources,
            input_pointer,
            output_pointer,
        } => {
            let sources: Vec<PriceSource> = sources
                .iter()
                .copied()
                .filter(|s| *s != PriceSource::Unknown)
                .collect();
            if sources.is_empty() {
                return Err(("sources", "Choose where the price comes from".into()));
            }
            let clean = |p: &Option<String>| {
                p.as_deref()
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(str::to_string)
            };
            let input_pointer = clean(input_pointer);
            let output_pointer = clean(output_pointer);
            if let Some(p) = &input_pointer {
                check_pointer(p).map_err(|e| ("input_pointer", e))?;
            }
            if let Some(p) = &output_pointer {
                check_pointer(p).map_err(|e| ("output_pointer", e))?;
            }
            if output_pointer.is_some() && input_pointer.is_none() {
                return Err((
                    "input_pointer",
                    "Set the input price path too (or neither)".into(),
                ));
            }
            Ok(Condition::ZeroPrice {
                sources,
                input_pointer,
                output_pointer,
            })
        }
        FreeModelCondition::Unsupported => Err((
            "type",
            "This rule type is not supported by this version".into(),
        )),
    }
}

const NO_LISTING: &str = "no listing data for this model";

impl Condition {
    fn eval(&self, input: &FreeModelInput) -> Outcome {
        match self {
            Condition::ModelId(m) => {
                let (hit, how) = match m {
                    IdMatcher::Exact { pattern, ci } => (
                        if *ci {
                            input.model_id.eq_ignore_ascii_case(pattern)
                        } else {
                            input.model_id == pattern
                        },
                        format!("id is \"{pattern}\""),
                    ),
                    IdMatcher::Glob { pattern, ci } => (
                        glob_match(pattern, input.model_id, *ci),
                        format!("id matches \"{pattern}\""),
                    ),
                    IdMatcher::Regex(re) => (
                        re.is_match(input.model_id),
                        format!("id matches /{}/", re.as_str()),
                    ),
                };
                if hit {
                    Outcome::Match(how)
                } else {
                    Outcome::NoMatch(format!("not: {how}"))
                }
            }
            Condition::Field {
                pointer,
                op,
                value,
                regex,
            } => {
                let Some(meta) = input.metadata else {
                    return Outcome::Unknown(NO_LISTING.into());
                };
                eval_field(meta, pointer, *op, value, regex.as_ref())
            }
            Condition::JsonPath(path) => {
                let Some(meta) = input.metadata else {
                    return Outcome::Unknown(NO_LISTING.into());
                };
                let wrapped = Value::Array(vec![meta.clone()]);
                if path.query(&wrapped).is_empty() {
                    Outcome::NoMatch("JSONPath filter does not match".into())
                } else {
                    Outcome::Match("JSONPath filter matches".into())
                }
            }
            Condition::ZeroPrice {
                sources,
                input_pointer,
                output_pointer,
            } => {
                for source in sources {
                    let price = match source {
                        PriceSource::PricingOverride => input.price_override.map(|p| {
                            (
                                p.input_per_million == 0.0 && p.output_per_million == 0.0,
                                "price set on the Models tab",
                            )
                        }),
                        PriceSource::ProviderListing => input.metadata.and_then(|m| {
                            listed_zero(m, input_pointer.as_deref(), output_pointer.as_deref())
                                .map(|zero| (zero, "price in the provider's listing"))
                        }),
                        PriceSource::Unknown => None,
                    };
                    if let Some((zero, what)) = price {
                        return if zero {
                            Outcome::Match(format!("$0 {what}"))
                        } else {
                            Outcome::NoMatch(format!("not $0 ({what})"))
                        };
                    }
                }
                Outcome::Unknown("no price known for this model".into())
            }
            Condition::Invalid => Outcome::Unknown("the rule has an error".into()),
        }
    }
}

/// Whether a listing entry prices the model at $0. `None` when it has no
/// usable price.
fn listed_zero(model: &Value, input: Option<&str>, output: Option<&str>) -> Option<bool> {
    let (input, output, request) = match input {
        Some(pointer) => (
            model.pointer(pointer)?,
            match output {
                Some(p) => Some(model.pointer(p)?),
                None => None,
            },
            None,
        ),
        None => {
            let (i, o) = listed_price_fields(model)?;
            (i, o, model.pointer("/pricing/request"))
        }
    };
    let zero = |v: &Value| price_value(v).map(|p| p == 0.0);
    let mut is_zero = zero(input)?;
    if let Some(o) = output {
        is_zero &= zero(o)?;
    }
    // A per-request fee makes a model with $0 tokens paid.
    if let Some(r) = request {
        is_zero &= zero(r).unwrap_or(false);
    }
    Some(is_zero)
}

fn as_number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok().filter(|f| f.is_finite()),
        _ => None,
    }
}

/// Equality that lets numbers and numeric strings, and booleans and
/// `"true"`/`"false"`, compare equal (listings are not consistent about it).
fn loose_eq(a: &Value, b: &Value) -> bool {
    if a == b {
        return true;
    }
    match (a, b) {
        (Value::Number(_), _) | (_, Value::Number(_)) => {
            matches!((as_number(a), as_number(b)), (Some(x), Some(y)) if x == y)
        }
        (Value::Bool(x), Value::String(s)) | (Value::String(s), Value::Bool(x)) => s
            .trim()
            .eq_ignore_ascii_case(if *x { "true" } else { "false" }),
        _ => false,
    }
}

fn show(v: &Value) -> String {
    let s = v.to_string();
    if s.len() > 60 {
        format!("{}…", s.chars().take(57).collect::<String>())
    } else {
        s
    }
}

fn eval_field(
    meta: &Value,
    pointer: &str,
    op: FieldOp,
    value: &Value,
    regex: Option<&Regex>,
) -> Outcome {
    let name = if pointer.is_empty() {
        "(model)"
    } else {
        pointer
    };
    let field = meta.pointer(pointer).filter(|v| !v.is_null());
    let Some(field) = field else {
        return if op == FieldOp::NotExists {
            Outcome::Match(format!("{name} is missing"))
        } else {
            Outcome::NoMatch(format!("{name} is missing"))
        };
    };
    let is = format!("{name} is {}", show(field));
    let hit = match op {
        FieldOp::Exists => true,
        FieldOp::NotExists => false,
        FieldOp::Equals => loose_eq(field, value),
        FieldOp::NotEquals => !loose_eq(field, value),
        FieldOp::Contains => match field {
            Value::String(s) => value.as_str().is_some_and(|v| s.contains(v)),
            Value::Array(items) => items.iter().any(|i| loose_eq(i, value)),
            _ => false,
        },
        FieldOp::Matches => {
            let text = match field {
                Value::String(s) => s.clone(),
                Value::Number(_) | Value::Bool(_) => field.to_string(),
                _ => return Outcome::NoMatch(is),
            };
            regex.is_some_and(|re| re.is_match(&text))
        }
        FieldOp::LessThan
        | FieldOp::LessOrEqual
        | FieldOp::GreaterThan
        | FieldOp::GreaterOrEqual => {
            let (Some(x), Some(y)) = (as_number(field), as_number(value)) else {
                return Outcome::NoMatch(is);
            };
            match op {
                FieldOp::LessThan => x < y,
                FieldOp::LessOrEqual => x <= y,
                FieldOp::GreaterThan => x > y,
                _ => x >= y,
            }
        }
        FieldOp::Unknown => false,
    };
    if hit {
        Outcome::Match(is)
    } else {
        Outcome::NoMatch(is)
    }
}

/// Glob match of the whole text: `*` matches any run of characters
/// (including none); everything else matches itself.
pub fn glob_match(pattern: &str, text: &str, case_insensitive: bool) -> bool {
    let fold = |s: &str| -> Vec<char> {
        if case_insensitive {
            s.chars().flat_map(char::to_lowercase).collect()
        } else {
            s.chars().collect()
        }
    };
    let (p, t) = (fold(pattern), fold(text));
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if pi < p.len() && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

fn usd(v: f64) -> String {
    if v == 0.0 {
        "$0".into()
    } else if v < 0.01 {
        format!("${v:.4}")
    } else {
        format!("${v:.2}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn free_models(rules: Vec<FreeModelRule>) -> FreeTierKind {
        FreeTierKind::FreeModelsOnly {
            free_model_patterns: vec![],
            excluded_models: vec![],
            rules,
            max_rpm: 0,
        }
    }

    fn rule(when: FreeModelCondition) -> FreeModelRule {
        FreeModelRule {
            enabled: true,
            effect: FreeModelRuleEffect::Include,
            when,
            label: None,
        }
    }

    fn exclude(when: FreeModelCondition) -> FreeModelRule {
        FreeModelRule {
            effect: FreeModelRuleEffect::Exclude,
            ..rule(when)
        }
    }

    fn id_rule(pattern: &str, syntax: ModelIdPatternSyntax, ci: bool) -> FreeModelRule {
        rule(FreeModelCondition::ModelId {
            pattern: pattern.into(),
            syntax,
            case_insensitive: ci,
        })
    }

    fn field(pointer: &str, op: FieldOp, value: Value) -> FreeModelCondition {
        FreeModelCondition::Field {
            pointer: pointer.into(),
            op,
            value,
        }
    }

    fn zero_price(sources: Vec<PriceSource>) -> FreeModelCondition {
        FreeModelCondition::ZeroPrice {
            sources,
            input_pointer: None,
            output_pointer: None,
        }
    }

    fn id(model_id: &str) -> FreeModelInput<'_> {
        FreeModelInput {
            model_id,
            ..Default::default()
        }
    }

    fn with_meta<'a>(model_id: &'a str, meta: &'a Value) -> FreeModelInput<'a> {
        FreeModelInput {
            model_id,
            metadata: Some(meta),
            price_override: None,
        }
    }

    fn free(kind: &FreeTierKind, input: FreeModelInput) -> bool {
        let ev = FreeTierEvaluator::new(kind);
        assert!(ev.errors().is_empty(), "{:?}", ev.errors());
        ev.decide(&input).is_free
    }

    #[test]
    fn globs_take_any_number_of_stars() {
        assert!(glob_match("*:free", "meta/llama:free", false));
        assert!(glob_match("meta/*", "meta/llama", false));
        assert!(glob_match("*llama*free*", "meta/llama-3-free-x", false));
        assert!(glob_match("a*b*c", "abc", false));
        assert!(glob_match("*", "", false));
        assert!(glob_match("exact", "exact", false));
        assert!(!glob_match("exact", "exactly", false));
        assert!(!glob_match("*:free", "x:free-ish", false));
        assert!(!glob_match("a*b*c", "acb", false));
        assert!(!glob_match("*-Free", "x-free", false));
        assert!(glob_match("*-Free", "x-free", true));
        // ":free" alone is an exact id, as it always was.
        assert!(!glob_match(":free", "x:free", false));
    }

    #[test]
    fn lists_come_before_rules_and_exclusions_first() {
        let kind = FreeTierKind::FreeModelsOnly {
            free_model_patterns: vec!["*:free".into(), "special".into()],
            excluded_models: vec!["bad:free".into()],
            rules: vec![id_rule("*", ModelIdPatternSyntax::Glob, false)],
            max_rpm: 0,
        };
        let ev = FreeTierEvaluator::new(&kind);
        let d = ev.decide(&id("bad:free"));
        assert!(!d.is_free);
        assert_eq!(d.decided_by, DecidedBy::ExcludedList);
        let d = ev.decide(&id("good:free"));
        assert!(d.is_free);
        assert_eq!(d.decided_by, DecidedBy::IncludedList);
        let d = ev.decide(&id("other"));
        assert!(d.is_free);
        assert_eq!((d.decided_by, d.rule_index), (DecidedBy::Rule, Some(0)));
        assert_eq!(d.reason, "Rule 1: id matches \"*\"");
    }

    #[test]
    fn first_enabled_matching_rule_decides() {
        let mut disabled = id_rule("a*", ModelIdPatternSyntax::Glob, false);
        disabled.enabled = false;
        let kind = free_models(vec![
            disabled,
            exclude(FreeModelCondition::ModelId {
                pattern: "ab".into(),
                syntax: ModelIdPatternSyntax::Exact,
                case_insensitive: false,
            }),
            id_rule("a*", ModelIdPatternSyntax::Glob, false),
        ]);
        let ev = FreeTierEvaluator::new(&kind);
        let d = ev.decide(&id("ab"));
        assert!(!d.is_free);
        assert_eq!(d.rule_index, Some(1));
        let d = ev.decide(&id("ac"));
        assert!(d.is_free);
        assert_eq!(d.rule_index, Some(2));
        let d = ev.decide(&id("b"));
        assert!(!d.is_free);
        assert_eq!(d.decided_by, DecidedBy::NoRule);
        // Match counts ignore order and the enabled switch.
        assert!(ev.rule_matches(0, &id("ac")));
        assert!(!ev.rule_matches(1, &id("ac")));
        assert_eq!(ev.rule_count(), 3);
    }

    #[test]
    fn model_id_syntaxes() {
        let k = |r| free_models(vec![r]);
        assert!(free(
            &k(id_rule("Model-A", ModelIdPatternSyntax::Exact, true)),
            id("model-a")
        ));
        assert!(!free(
            &k(id_rule("Model-A", ModelIdPatternSyntax::Exact, false)),
            id("model-a")
        ));
        assert!(free(
            &k(id_rule(r"-free$", ModelIdPatternSyntax::Regex, false)),
            id("x-free")
        ));
        assert!(!free(
            &k(id_rule(r"-free$", ModelIdPatternSyntax::Regex, false)),
            id("x-Free")
        ));
        assert!(free(
            &k(id_rule(r"-free$", ModelIdPatternSyntax::Regex, true)),
            id("x-Free")
        ));
        // Regex searches; anchors are the user's.
        assert!(free(
            &k(id_rule("free", ModelIdPatternSyntax::Regex, false)),
            id("a-free-b")
        ));
    }

    #[test]
    fn field_ops() {
        let meta = json!({
            "tier": "free",
            "rank": 3,
            "price": "0.5",
            "beta": true,
            "tags": ["fast", "free-tier"],
            "nested": {"plan": "Free Plan"},
            "nothing": null
        });
        let cases: Vec<(FreeModelCondition, bool)> = vec![
            (field("/tier", FieldOp::Equals, json!("free")), true),
            (field("/tier", FieldOp::Equals, json!("Free")), false),
            (field("/tier", FieldOp::NotEquals, json!("paid")), true),
            (field("/rank", FieldOp::Equals, json!("3")), true),
            (field("/rank", FieldOp::Equals, json!(3.0)), true),
            (field("/beta", FieldOp::Equals, json!("true")), true),
            (field("/tier", FieldOp::Exists, Value::Null), true),
            (field("/nothing", FieldOp::Exists, Value::Null), false),
            (field("/nothing", FieldOp::NotExists, Value::Null), true),
            (field("/absent", FieldOp::NotExists, Value::Null), true),
            (field("/absent", FieldOp::Equals, json!("x")), false),
            (field("/absent", FieldOp::NotEquals, json!("x")), false),
            (field("/tags", FieldOp::Contains, json!("fast")), true),
            (
                field("/nested/plan", FieldOp::Contains, json!("Free")),
                true,
            ),
            (
                field("/nested/plan", FieldOp::Matches, json!("(?i)^free")),
                true,
            ),
            (field("/rank", FieldOp::Matches, json!("^3$")), true),
            (field("/rank", FieldOp::LessThan, json!(5)), true),
            (field("/rank", FieldOp::LessOrEqual, json!("3")), true),
            (field("/price", FieldOp::GreaterThan, json!(0)), true),
            (field("/price", FieldOp::GreaterOrEqual, json!(1)), false),
            (field("/tier", FieldOp::LessThan, json!(1)), false),
        ];
        for (cond, want) in cases {
            let kind = free_models(vec![rule(cond.clone())]);
            assert_eq!(free(&kind, with_meta("m", &meta)), want, "{cond:?}");
        }
    }

    #[test]
    fn metadata_rules_fail_closed_without_a_listing() {
        let include = free_models(vec![rule(field("/tier", FieldOp::NotExists, Value::Null))]);
        assert!(!free(&include, id("m")));
        // An exclude rule applies when it cannot tell.
        let kind = free_models(vec![
            exclude(field("/tier", FieldOp::Equals, json!("paid"))),
            id_rule("*", ModelIdPatternSyntax::Glob, false),
        ]);
        let ev = FreeTierEvaluator::new(&kind);
        let d = ev.decide(&id("m"));
        assert!(!d.is_free);
        assert_eq!(d.rule_index, Some(0));
        assert!(d.reason.contains("no listing data"), "{}", d.reason);
        let meta = json!({"tier": "free"});
        assert!(ev.decide(&with_meta("m", &meta)).is_free);
        assert!(ev.needs_metadata());
        assert!(!FreeTierEvaluator::new(&free_models(vec![id_rule(
            "*",
            ModelIdPatternSyntax::Glob,
            false
        )]))
        .needs_metadata());
    }

    #[test]
    fn json_path_filters_the_model() {
        let kind = |e: &str| {
            free_models(vec![rule(FreeModelCondition::JsonPath {
                expression: e.into(),
            })])
        };
        let meta = json!({"tier": "free", "pricing": {"prompt": "0"}, "tags": ["a", "b"]});
        assert!(free(&kind("@.tier == 'free'"), with_meta("m", &meta)));
        assert!(free(
            &kind("?@.pricing.prompt == '0'"),
            with_meta("m", &meta)
        ));
        assert!(free(&kind("@.tags[?@ == 'b']"), with_meta("m", &meta)));
        assert!(free(&kind("match(@.tier, 'f.*')"), with_meta("m", &meta)));
        assert!(!free(&kind("@.tier == 'paid'"), with_meta("m", &meta)));
        assert!(!free(&kind("@.tier == 'free'"), id("m")));
    }

    #[test]
    fn zero_price_sources_in_order() {
        let listed_free = json!({"pricing": {"prompt": "0", "completion": "0"}});
        let listed_paid = json!({"pricing": {"prompt": "0.000001", "completion": "0.000002"}});
        let request_fee = json!({"pricing": {"prompt": "0", "completion": "0", "request": "0.01"}});
        let variable = json!({"pricing": {"prompt": "-1", "completion": "-1"}});
        let free_override = ModelPricingOverride {
            input_per_million: 0.0,
            output_per_million: 0.0,
        };
        let paid_override = ModelPricingOverride {
            input_per_million: 1.0,
            output_per_million: 0.0,
        };
        let both = free_models(vec![rule(zero_price(vec![
            PriceSource::PricingOverride,
            PriceSource::ProviderListing,
        ]))]);
        let listing = free_models(vec![rule(zero_price(vec![PriceSource::ProviderListing]))]);

        assert!(free(&listing, with_meta("m", &listed_free)));
        assert!(!free(&listing, with_meta("m", &listed_paid)));
        assert!(!free(&listing, with_meta("m", &request_fee)));
        assert!(!free(&listing, with_meta("m", &variable)));
        assert!(!free(&listing, id("m")));
        // The override comes first.
        let input = FreeModelInput {
            model_id: "m",
            metadata: Some(&listed_paid),
            price_override: Some(&free_override),
        };
        assert!(free(&both, input));
        let input = FreeModelInput {
            model_id: "m",
            metadata: Some(&listed_free),
            price_override: Some(&paid_override),
        };
        assert!(!free(&both, input));
        // Listing-only rules ignore overrides.
        assert!(!free(
            &listing,
            FreeModelInput {
                model_id: "m",
                metadata: None,
                price_override: Some(&free_override),
            }
        ));
    }

    #[test]
    fn zero_price_at_custom_paths() {
        let kind = free_models(vec![rule(FreeModelCondition::ZeroPrice {
            sources: vec![PriceSource::ProviderListing],
            input_pointer: Some("/cost/in".into()),
            output_pointer: Some("/cost/out".into()),
        })]);
        assert!(free(
            &kind,
            with_meta("m", &json!({"cost": {"in": 0, "out": "0.0"}}))
        ));
        assert!(!free(
            &kind,
            with_meta("m", &json!({"cost": {"in": 0, "out": 2}}))
        ));
        assert!(!free(&kind, with_meta("m", &json!({"cost": {"in": 0}}))));
        // Free.ai style: only an input price.
        let listing = free_models(vec![rule(zero_price(vec![PriceSource::ProviderListing]))]);
        assert!(free(
            &listing,
            with_meta("m", &json!({"pricing": {"input": 0}}))
        ));
    }

    #[test]
    fn invalid_rules_are_reported_and_never_free() {
        let long = "a".repeat(MAX_PATTERN_LEN + 1);
        let kind = FreeTierKind::FreeModelsOnly {
            free_model_patterns: vec![long.clone(), " ".into()],
            excluded_models: vec![],
            rules: vec![
                id_rule("(", ModelIdPatternSyntax::Regex, false),
                id_rule("", ModelIdPatternSyntax::Glob, false),
                rule(field("tier", FieldOp::Equals, json!("free"))),
                rule(field("/tier", FieldOp::Equals, Value::Null)),
                rule(field("/n", FieldOp::LessThan, json!("many"))),
                rule(FreeModelCondition::JsonPath {
                    expression: "@.tier ==".into(),
                }),
                rule(FreeModelCondition::JsonPath {
                    expression: "a".repeat(MAX_JSON_PATH_LEN + 1),
                }),
                rule(zero_price(vec![PriceSource::Unknown])),
                rule(FreeModelCondition::Unsupported),
                id_rule("a{1000}{1000}", ModelIdPatternSyntax::Regex, false),
            ],
            max_rpm: 0,
        };
        let ev = FreeTierEvaluator::new(&kind);
        let errors = ev.errors();
        let fields: Vec<(Option<usize>, &str)> = errors
            .iter()
            .map(|e| (e.rule_index, e.field.as_str()))
            .collect();
        assert_eq!(
            fields,
            vec![
                (None, "free_model_patterns"),
                (Some(0), "pattern"),
                (Some(1), "pattern"),
                (Some(2), "pointer"),
                (Some(3), "value"),
                (Some(4), "value"),
                (Some(5), "expression"),
                (Some(6), "expression"),
                (Some(7), "sources"),
                (Some(8), "type"),
                (Some(9), "pattern"),
            ]
        );
        assert!(
            errors[10].message.contains("too complex"),
            "{}",
            errors[10].message
        );
        assert_eq!(
            errors[1].message,
            "Invalid regular expression: unclosed group"
        );
        assert!(!ev.decide(&id(&long)).is_free);
        assert!(!ev.decide(&id("(")).is_free);
        // A broken exclude rule excludes.
        let kind = free_models(vec![
            exclude(FreeModelCondition::Unsupported),
            id_rule("*", ModelIdPatternSyntax::Glob, false),
        ]);
        assert!(!FreeTierEvaluator::new(&kind).decide(&id("m")).is_free);
        assert_eq!(validate(&kind).len(), 1);
        // An empty value is no value.
        let empty = free_models(vec![rule(field("/t", FieldOp::Equals, json!("")))]);
        assert_eq!(validate(&empty)[0].field, "value");
    }

    #[test]
    fn unknown_effect_acts_as_exclude() {
        let mut r = id_rule("*", ModelIdPatternSyntax::Glob, false);
        r.effect = FreeModelRuleEffect::Unknown;
        let kind = FreeTierKind::FreeModelsOnly {
            free_model_patterns: vec![],
            excluded_models: vec![],
            rules: vec![r, id_rule("*", ModelIdPatternSyntax::Glob, false)],
            max_rpm: 0,
        };
        let ev = FreeTierEvaluator::new(&kind);
        assert_eq!(ev.errors().len(), 1);
        assert!(!ev.decide(&id("m")).is_free);
    }

    #[test]
    fn whole_provider_kinds() {
        let paid = ModelPricingOverride {
            input_per_million: 0.0,
            output_per_million: 0.5,
        };
        let zero = ModelPricingOverride {
            input_per_million: 0.0,
            output_per_million: 0.0,
        };
        let priced = FreeModelInput {
            model_id: "m",
            metadata: None,
            price_override: Some(&paid),
        };
        let zeroed = FreeModelInput {
            price_override: Some(&zero),
            ..priced
        };
        for kind in [FreeTierKind::AlwaysFreeLocal, FreeTierKind::Subscription] {
            let ev = FreeTierEvaluator::new(&kind);
            assert!(ev.decide(&id("m")).is_free);
            assert!(ev.decide(&zeroed).is_free);
            let d = ev.decide(&priced);
            assert!(!d.is_free);
            assert_eq!(d.decided_by, DecidedBy::PriceOverride);
        }
        let none = FreeTierEvaluator::new(&FreeTierKind::None).decide(&id("m"));
        assert_eq!(
            (none.is_free, none.decided_by),
            (false, DecidedBy::NoFreeTier)
        );
        let credits = FreeTierKind::CreditBased {
            budget_usd: 1.0,
            reset_period: lr_config::FreeTierResetPeriod::Monthly,
            detection: lr_config::CreditDetection::LocalOnly,
        };
        let d = FreeTierEvaluator::new(&credits).decide(&priced);
        assert_eq!((d.is_free, d.decided_by), (true, DecidedBy::Provider));
    }
}
