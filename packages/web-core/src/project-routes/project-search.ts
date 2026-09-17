import { zodValidator } from '@tanstack/zod-adapter';
import { z } from 'zod';

/**
 * 项目相关路由的 query 参数。
 *
 * 全部 `.optional()` 且只做「是不是字符串」这一层校验：
 * URL 是用户可以随手改的，schema 抛错会把整页打死。
 * 真正的取值校验与归一在 `features/kanban/model/kanbanUrlState.ts` 里做
 * （非法值丢弃 / 回落默认，绝不抛错）。
 *
 * 由 8 个 local-web 路由与 6 个 remote-web 路由共用；
 * 原先是空对象 `z.object({})`（会把所有 search 参数丢掉），放宽是向后兼容的。
 */
export const projectSearchSchema = z.object({
  /** 关键字 */
  q: z.string().optional(),
  /** 优先级，逗号分隔 */
  priority: z.string().optional(),
  /** 负责人，逗号分隔，可含 `__self__` / `unassigned` */
  assignee: z.string().optional(),
  /** 标签，逗号分隔 */
  tag: z.string().optional(),
  /** 排序字段 */
  sort: z.string().optional(),
  /** 排序方向 */
  dir: z.string().optional(),
});

export type ProjectSearch = z.infer<typeof projectSearchSchema>;

export const projectSearchValidator = zodValidator(projectSearchSchema);
