# Critical Bugs Found in Router Module

Date: 2026-01-15
Severity: HIGH
Affected File: `src-tauri/src/router/mod.rs`

## Bug #1: Response Lost When Usage Recording Fails ⚠️ CRITICAL

### Severity: HIGH
**Location**: Lines 481-483, 100-102

### Description
After a provider successfully completes a request, if the `record_api_key_usage()` call fails, the `?` operator propagates the error and **discards the successful response**. This means the user doesn't receive the completion they paid for.

### Vulnerable Code
```rust
// Line 466-472: Provider completes successfully
let response = provider_instance.complete(modified_request).await?;

// Line 474-483: Usage recording
let usage = UsageInfo {
    input_tokens: response.usage.prompt_tokens as u64,
    output_tokens: response.usage.completion_tokens as u64,
    cost_usd: 0.0,
};

self.rate_limiter
    .record_api_key_usage(api_key_id, &usage)
    .await?;  // ❌ If this fails, response is lost!

Ok(response)
```

### Impact
1. **User Experience**: User doesn't receive the response even though provider succeeded
2. **Billing**: Provider APIs were charged but user gets error
3. **Security**: Failed recordings don't count toward rate limits → potential abuse
4. **Data Loss**: Successful provider responses are discarded
5. **Occurs in TWO places**:
   - `complete()` at line 481-483
   - `complete_with_prioritized_list()` at line 100-102

### Reproduction Scenario
1. API key makes a request
2. Provider (e.g., OpenAI) successfully completes the request ($0.02 charged)
3. Rate limiter storage fails (disk full, database error, etc.)
4. `record_api_key_usage()` returns error
5. `?` operator propagates error
6. User receives error instead of their $0.02 response

### Recommended Fix
```rust
// Record usage for rate limiting
let usage = UsageInfo {
    input_tokens: response.usage.prompt_tokens as u64,
    output_tokens: response.usage.completion_tokens as u64,
    cost_usd: 0.0, // TODO: Calculate actual cost
};

// Log error but don't fail the request
if let Err(e) = self.rate_limiter.record_api_key_usage(api_key_id, &usage).await {
    error!(
        "Failed to record usage for API key '{}': {}. Request succeeded but usage not tracked.",
        api_key_id, e
    );
    // Optionally emit a monitoring alert here
}

Ok(response)
```

---

## Bug #2: Streaming Doesn't Support PrioritizedList Retry Logic

### Severity: MEDIUM
**Location**: Lines 601-610

### Description
The `stream_complete()` function doesn't implement retry logic for PrioritizedList strategy. It only tries the first model in the list.

### Vulnerable Code
```rust
ActiveRoutingStrategy::PrioritizedList => {
    // Use first model in prioritized list
    if let Some((first_provider, first_model)) = config.prioritized_models.first() {
        (first_provider.clone(), first_model.clone())
    } else {
        return Err(AppError::Router(
            "Prioritized List strategy is active but no models are configured".to_string()
        ));
    }
}
```

### Impact
1. **Inconsistent Behavior**: Non-streaming retries, streaming doesn't
2. **Reliability**: No automatic failover for streaming requests
3. **User Confusion**: Same strategy works differently for stream vs non-stream

### Recommended Fix
Implement `stream_complete_with_prioritized_list()` that wraps the stream and handles failures by retrying with the next provider in the list.

---

## Bug #3: Streaming Requests Don't Record Usage

### Severity: HIGH (Security/Billing)
**Location**: Lines 738-740

### Description
Streaming requests completely skip usage recording, as admitted by the TODO comment.

### Vulnerable Code
```rust
// TODO: Record usage after stream completes
// This is challenging because we need to count tokens as they stream
// For now, usage recording is skipped for streaming requests

Ok(stream)
```

### Impact
1. **Rate Limiting Bypass**: Users can abuse streaming to avoid all rate limits
2. **No Cost Tracking**: Can't bill or track costs for streaming
3. **Security Issue**: Malicious users can use unlimited tokens via streaming
4. **Monitoring Blind Spot**: No visibility into streaming usage

### Recommended Fix
1. Wrap the returned stream in a counting wrapper
2. When stream completes, record the total usage
3. Handle stream errors appropriately

```rust
// Wrap stream to count tokens and record usage when done
let wrapped_stream = count_and_record_stream(
    stream,
    api_key_id.to_string(),
    self.rate_limiter.clone(),
);

Ok(Box::pin(wrapped_stream))
```

---

## Bug #4: Dead Code - Unreachable PrioritizedList Case

### Severity: LOW (Code Quality)
**Location**: Lines 301-313

### Description
The PrioritizedList match arm in the routing logic is unreachable because there's an early return at lines 224-226.

### Code
```rust
// Lines 209-227: Early return for PrioritizedList
if let Some(ref config) = routing_config {
    if config.active_strategy == ActiveRoutingStrategy::PrioritizedList {
        // ... validation ...
        return self
            .complete_with_prioritized_list(api_key_id, &config.prioritized_models, request)
            .await;
    }
}

// Lines 301-313: UNREACHABLE CODE
ActiveRoutingStrategy::PrioritizedList => {
    // This code is never executed!
    if let Some((first_provider, first_model)) = config.prioritized_models.first() {
        (first_provider.clone(), first_model.clone())
    } else {
        return Err(AppError::Router(...));
    }
}
```

### Impact
- Confusing code maintenance
- False sense of coverage

### Recommended Fix
Remove the unreachable match arm or add a comment explaining it's a safety fallback.

---

## Suggested Test Coverage

Add tests for:
1. ✅ Test that usage recording failure doesn't fail the request
2. ✅ Test that response is returned even when rate limiter storage fails
3. ⬜ Test streaming with PrioritizedList retry
4. ⬜ Test streaming usage recording
5. ⬜ Integration test with failing rate limiter

---

## Priority

1. **Fix Bug #1 immediately** - Critical data loss and billing issue
2. **Fix Bug #3** - Security issue, rate limit bypass
3. **Fix Bug #2** - Feature parity and reliability
4. **Clean up Bug #4** - Code quality

---

## Notes

All bugs were found through careful code review of the routing logic on 2026-01-15.
Testing was deferred due to ProviderRegistry architecture requiring factory-based setup.
