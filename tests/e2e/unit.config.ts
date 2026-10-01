import { defineConfig } from '@playwright/test'

// Pure TypeScript regressions: no browser, app, global setup, network, or GPU.
export default defineConfig({
  testDir: './unit',
  fullyParallel: true,
  workers: 2,
  timeout: 10_000,
  reporter: 'list',
  outputDir: '../../test-results/unit',
})
