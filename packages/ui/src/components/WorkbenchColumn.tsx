import type { ReactNode } from 'react';

export interface WorkbenchColumnProps {
  testId: string;
  title: string;
  count: number;
  emptyText: string;
  footer?: ReactNode;
  children?: ReactNode;
}

/** 工作台三栏之一（设计文档 §8.2）。空栏写明为什么空。 */
export function WorkbenchColumn({
  testId,
  title,
  count,
  emptyText,
  footer,
  children,
}: WorkbenchColumnProps) {
  return (
    <section data-testid={testId} className="flex min-w-0 flex-col gap-half">
      <header className="flex items-center gap-half">
        <h2 className="m-0 text-sm font-medium text-high">{title}</h2>
        <span className="font-ibm-plex-mono text-sm text-low">{count}</span>
      </header>
      <div className="flex flex-col gap-half">
        {count === 0 ? (
          <p className="m-0 rounded-sm border border-dashed border-border p-base text-center text-sm text-low">
            {emptyText}
          </p>
        ) : (
          children
        )}
      </div>
      {footer}
    </section>
  );
}
