import path from 'node:path';
import { defineConfig } from '@playwright/test';
import {
  ASSET_DIR,
  BACKEND_PORT,
  BACKEND_URL,
  FRONTEND_PORT,
  FRONTEND_URL,
  REPOS_DIR,
  REPO_ROOT,
} from './support/env';

const isCI = !!process.env.CI;

/**
 * 真后端（qa-mode 模拟执行器）+ 真前端（vite dev），不打桩网络层（设计文档 §10）。
 *
 * - 只用 Playwright 自带的无头 Chromium：`browserName: 'chromium'`，**不设 channel**，
 *   不会连接或启动本机 Google Chrome。
 * - 后端只有一个实例，所有用例串行（workers: 1）；每个用例自己建项目与仓库，互不干扰。
 * - 数据目录 `VK_ASSET_DIR` 是本次运行新建的临时目录（`support/env.ts`），库每次都是干净的。
 */
export default defineConfig({
  testDir: './tests',
  outputDir: './test-results',
  snapshotPathTemplate:
    '{testDir}/__screenshots__/{testFilePath}/{arg}-{platform}{ext}',
  fullyParallel: false,
  workers: 1,
  timeout: 6 * 60_000,
  retries: isCI ? 1 : 0,
  expect: {
    timeout: 15_000,
    toHaveScreenshot: { maxDiffPixelRatio: 0.01, animations: 'disabled' },
  },
  reporter: isCI
    ? [
        ['github'],
        ['html', { open: 'never', outputFolder: 'playwright-report' }],
      ]
    : [
        ['list'],
        ['html', { open: 'never', outputFolder: 'playwright-report' }],
      ],
  use: {
    baseURL: FRONTEND_URL,
    browserName: 'chromium',
    headless: true,
    locale: 'zh-CN',
    timezoneId: 'Asia/Shanghai',
    viewport: { width: 1440, height: 900 },
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'off',
  },
  projects: [{ name: 'chromium', use: { browserName: 'chromium' } }],
  webServer: [
    {
      command: `mkdir -p "${ASSET_DIR}" "${REPOS_DIR}" && cargo run -p server --bin server --features qa-mode`,
      cwd: REPO_ROOT,
      url: `${BACKEND_URL}/api/health`,
      // 首次要编译 qa-mode 后端，给足时间
      timeout: 20 * 60_000,
      reuseExistingServer: !isCI,
      stdout: 'pipe',
      stderr: 'pipe',
      env: {
        BACKEND_PORT: String(BACKEND_PORT),
        HOST: '127.0.0.1',
        VK_ASSET_DIR: ASSET_DIR,
        VK_ALLOWED_ORIGINS: FRONTEND_URL,
        DISABLE_WORKTREE_CLEANUP: '1',
        RUST_LOG: 'info',
      },
    },
    {
      command: `pnpm exec vite --port ${FRONTEND_PORT} --strictPort`,
      cwd: path.join(REPO_ROOT, 'packages/local-web'),
      url: FRONTEND_URL,
      timeout: 2 * 60_000,
      reuseExistingServer: !isCI,
      env: {
        FRONTEND_PORT: String(FRONTEND_PORT),
        BACKEND_PORT: String(BACKEND_PORT),
        VITE_OPEN: 'false',
        // 与根 `dev` 脚本一致：不连共享云端 API
        VITE_VK_SHARED_API_BASE: '',
        // vite 代理目标是 http://localhost:<BACKEND_PORT>，Node ≥ 17 可能把
        // localhost 解析成 ::1，而后端只听 127.0.0.1（计划 §4 V5）
        NODE_OPTIONS: '--dns-result-order=ipv4first',
      },
    },
  ],
});
