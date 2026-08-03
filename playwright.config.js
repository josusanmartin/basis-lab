const { defineConfig, devices } = require('@playwright/test');
const externalDeployment = process.env.LIVE_DEPLOYED_BASE_URL || process.env.DEPLOYED_BASE_URL;

module.exports = defineConfig({
  testDir: './tests/e2e',
  timeout: 45_000,
  expect: { timeout: 20_000 },
  fullyParallel: false,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [['line'], ['html', { open: 'never' }]] : 'line',
  use: {
    baseURL: 'http://127.0.0.1:8080',
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure'
  },
  webServer: externalDeployment ? undefined : {
    command: 'cargo run --release',
    url: 'http://127.0.0.1:8080/api/v1/health',
    reuseExistingServer: !process.env.CI,
    timeout: 180_000,
    env: { RUST_LOG: 'basis_lab=warn,tower_http=warn' }
  },
  projects: [
    { name: 'desktop-chromium', use: { ...devices['Desktop Chrome'] } },
    { name: 'mobile-chromium', use: { ...devices['Pixel 7'] } }
  ]
});
