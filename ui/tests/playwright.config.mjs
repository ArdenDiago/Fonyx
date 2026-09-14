import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: '.',
  testMatch: '*.spec.mjs',
  timeout: 30_000,
  fullyParallel: false, // all tests share one live daemon's state
  reporter: 'list',
  use: {
    baseURL: 'http://localhost:4571',
    headless: true,
  },
  webServer: {
    command: 'node bridge.mjs',
    url: 'http://localhost:4571',
    reuseExistingServer: true,
    timeout: 15_000,
  },
});
