# Dashboard monitor revamp

## Problems
- The event list and event detail overlap on the Dashboard: the list has a
  capped height, the detail flows below it in the page scroll, and the detail's
  own `h-full` layout fights the unconstrained page.
- The traffic graph is tall and decorative (gradients, 38px total, 190px chart).
- The graph is not live: it polls every 15s and does not show in-flight requests.
- The current minute never shows any requests (off by one).
- The event detail is a single long scroll with collapsed disclosures that dump
  raw JSON; it does not say which API the client spoke (Chat Completions,
  Responses, Messages, …) or which API LocalRouter spoke upstream, nor whether a
  translation happened.

## Plan
1. Tasks tracked in the session todo list.
2. Layout: the Dashboard fills the viewport without a page scroll. A compact
   traffic strip sits on top; the monitor fills the rest. Selecting an event opens
   a resizable split (drag handle, restored from the pre-ac4fbd4e layout) with the
   detail docked at the bottom or on the right; the dock side and split sizes are
   saved in localStorage.
3. Graph: ~110px stacked chart, no gradients, a live x-axis that ends at "now",
   refreshed on monitor events (debounced) and every few seconds for short ranges.
   In-flight requests are drawn at the live edge with a pulsing marker and count.
4. Fix the minute-bucket off-by-one at its root (backend).
5. Backend: record the client API and the upstream API on `LlmCall` events so the
   UI can show "Responses API → Chat Completions (translated)".
6. Event detail: tabs. "Exchange" shows request and response side by side;
   further tabs: "Details" (API, translation, model, provider, tokens, cost,
   timing, client, IDs as a readable property sheet), "Routing", "Settings &
   tools", "Transformations", and "Raw" (collapsible, syntax-highlighted JSON
   tree with copy). Non-LLM events get the same structure.
7. Update demo mocks and TypeScript types; update browser tests.

## Final steps
1. Plan review against the implementation.
2. Test coverage review (unit specs for new helpers, Rust tests for new fields
   and the bucket fix, browser specs for tabs and the split).
3. Bug hunt.
4. Commit and push.
