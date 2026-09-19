import { createFileRoute, redirect } from '@tanstack/react-router';
import { isLocalPersonalMode } from '@/shared/lib/local/runtimeMode';
import { WorkbenchPage } from '@/pages/workbench/WorkbenchPage';

/** 工作台（设计文档 §8.2）。只在个人版存在，团队版送回首页。 */
export const Route = createFileRoute('/_app/home')({
  beforeLoad: () => {
    if (!isLocalPersonalMode()) {
      throw redirect({ to: '/' });
    }
  },
  component: WorkbenchPage,
});
