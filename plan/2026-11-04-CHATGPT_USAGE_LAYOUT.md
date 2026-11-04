# Usage chart overflow

- [x] Inspect usage layout and reproduce the cause: 24 hourly bars overflow a fixed 112px chart.
- [x] Keep compact charts beside meters; place dense charts below with spaced axis labels and all hourly bars retained.
- [x] Add browser regression covering 24-hour, five-hour and weekly charts at narrow and wide card sizes.
- [x] Plan Review: compare implementation against the intended layout and accessibility.
- [x] Test Coverage Review: cover containment, meter separation, labels and compact windows.
- [x] Bug Hunt: inspect sizing and label edge cases.
- [x] Run desktop unit/build and shared website build checks.
- [x] Commit and push, merge with --no-ff into master and push master (`a8af112b`).

## Validation and review

- Plan review, coverage review and bug hunt complete. Dense windows use the full card width below the meter; all bars and their screen-reader summary remain present, while axis labels appear every three hours for a daily window.
- Browser regressions pass in Chrome at 300px, 360px and 720px card widths, covering weekly, daily and five-hour charts, containment, meter separation and label spacing. Rendered screenshots inspected.
- The 300px regression fails against the original component on chart containment, confirming it catches the reported overflow.
- All 142 unit tests pass. Desktop build and website build:check pass (existing bundle-size warnings). No Rust changes.
