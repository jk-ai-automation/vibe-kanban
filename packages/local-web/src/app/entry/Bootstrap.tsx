import React from 'react';
import ReactDOM from 'react-dom/client';
import * as Sentry from '@sentry/react';
import { ClickToComponent } from 'click-to-react-component';
import { QueryClientProvider } from '@tanstack/react-query';
import posthog from 'posthog-js';
import { PostHogProvider } from 'posthog-js/react';
import App from '@web/app/entry/App';
import { CrashScreen } from '@vibe/ui/components/CrashScreen';
import '@/i18n';
import { router } from '@web/app/router';
import { oauthApi } from '@/shared/lib/api';
import { tokenManager } from '@/shared/lib/auth/tokenManager';
import { configureAuthRuntime } from '@/shared/lib/auth/runtime';
import { fetchBootstrap } from '@/shared/lib/local/bootstrapApi';
import { applyBootstrap } from '@/shared/lib/local/runtimeMode';
import '@/shared/types/modals';
import { queryClient } from '@/shared/lib/queryClient';
import { isTauriApp } from '@/shared/lib/platform';
import { initZoom, zoomIn, zoomOut, zoomReset } from '@/shared/lib/zoom';

if (import.meta.env.VITE_SENTRY_DSN) {
  Sentry.init({
    dsn: import.meta.env.VITE_SENTRY_DSN,
    tracesSampleRate: 1.0,
    environment: import.meta.env.MODE === 'development' ? 'dev' : 'production',
    integrations: [Sentry.tanstackRouterBrowserTracingIntegration(router)],
  });
  Sentry.setTag('source', 'frontend');
}

if (
  import.meta.env.VITE_POSTHOG_API_KEY &&
  import.meta.env.VITE_POSTHOG_API_ENDPOINT
) {
  posthog.init(import.meta.env.VITE_POSTHOG_API_KEY, {
    api_host: import.meta.env.VITE_POSTHOG_API_ENDPOINT,
    capture_pageview: false,
    capture_pageleave: true,
    capture_performance: true,
    autocapture: false,
    opt_out_capturing_by_default: true,
  });
} else {
  console.warn(
    'PostHog API key or endpoint not set. Analytics will be disabled.'
  );
}

// In the Tauri desktop app, implement custom zoom (Cmd/Ctrl + =/–/0) via root
// font-size scaling and block trackpad/touchpad pinch-to-zoom.
if (isTauriApp()) {
  initZoom();

  document.addEventListener('keydown', (e) => {
    const mod = e.metaKey || e.ctrlKey;
    if (!mod) return;

    if (e.key === '=' || e.key === '+') {
      e.preventDefault();
      zoomIn();
    } else if (e.key === '-') {
      e.preventDefault();
      zoomOut();
    } else if (e.key === '0') {
      e.preventDefault();
      zoomReset();
    }
  });

  document.addEventListener(
    'wheel',
    (e) => {
      if (e.ctrlKey) e.preventDefault();
    },
    { passive: false }
  );
  document.addEventListener('gesturestart', (e) => e.preventDefault());
  document.addEventListener('gesturechange', (e) => e.preventDefault());
}

configureAuthRuntime({
  getToken: () => tokenManager.getToken(),
  triggerRefresh: () => tokenManager.triggerRefresh(),
  registerShape: (shape) => tokenManager.registerShape(shape),
  getCurrentUser: () => oauthApi.getCurrentUser(),
});

const hasSharedApiBase = Boolean(import.meta.env.VITE_VK_SHARED_API_BASE);

/**
 * 运行时模式与数据源：由 `GET /api/local-auth/bootstrap` 决定，
 * 不再依赖构建期的 `VITE_VK_DATA_SOURCE`（该变量全仓库无人声明，已删除）。
 *
 * `VITE_VK_SHARED_API_BASE` 仍然保留：个人模式下它决定数据从云端还是本机取，
 * 官方云端构建靠它，删掉会让现有构建回退。团队模式一律本机。
 */
async function configureRuntime(): Promise<void> {
  try {
    applyBootstrap(await fetchBootstrap(), { hasSharedApiBase });
  } catch (error) {
    // 后端没起来、或者是不认识 /api/local-auth/bootstrap 的旧后端：
    // 退回构建期的判断，按个人版渲染。这里**不能**渲染崩溃页——个人版
    // 在后端短暂不可用时本来也只是各页面报错，不该整站白屏。
    console.warn(
      '[bootstrap] Falling back to personal mode; /api/local-auth/bootstrap failed:',
      error
    );
    applyBootstrap(
      {
        mode: 'personal',
        require_login: false,
        authenticated: true,
        needs_setup: false,
        providers: [],
        allow_oauth_signup: false,
      },
      { hasSharedApiBase }
    );
  }
}

async function main(): Promise<void> {
  await configureRuntime();

  ReactDOM.createRoot(document.getElementById('root')!).render(
    <React.StrictMode>
      <QueryClientProvider client={queryClient}>
        <PostHogProvider client={posthog}>
          <Sentry.ErrorBoundary
            fallback={({ error, componentStack }) => (
              <CrashScreen
                error={error instanceof Error ? error : undefined}
                componentStack={componentStack}
              />
            )}
            showDialog
          >
            <ClickToComponent />
            <App />
          </Sentry.ErrorBoundary>
        </PostHogProvider>
      </QueryClientProvider>
    </React.StrictMode>
  );
}

void main();
