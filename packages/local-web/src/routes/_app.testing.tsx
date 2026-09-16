import { createFileRoute } from '@tanstack/react-router';
import { TestingPlaceholderPage } from '@/pages/testing/TestingPlaceholderPage';

export const Route = createFileRoute('/_app/testing')({
  component: TestingPlaceholderPage,
});
