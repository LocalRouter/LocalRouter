import { defineConfig } from '@playwright/test'
import demoConfig from './home-activity.config'

export default defineConfig({
  ...demoConfig,
  testMatch: 'decision-routing.spec.ts',
  outputDir: '../../test-results/decision-routing',
})
