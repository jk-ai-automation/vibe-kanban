import { createFileRoute, redirect } from '@tanstack/react-router';
import { isLocalPersonalMode } from '@/shared/lib/local/runtimeMode';
import { DocsPlaceholderPage } from '@/pages/docs/DocsPlaceholderPage';

export const Route = createFileRoute('/_app/docs')({
  beforeLoad: () => {
    if (!isLocalPersonalMode()) {
      throw redirect({ to: '/' });
    }
  },
  component: DocsPlaceholderPage,
});
