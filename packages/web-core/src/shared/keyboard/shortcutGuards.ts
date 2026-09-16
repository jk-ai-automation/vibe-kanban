/**
 * 单键快捷键的兜底护栏。
 *
 * `HotkeysProvider` 把 `kanban` scope 设成常驻激活
 * （`packages/local-web/src/app/entry/App.tsx:48-55`），所以看板的单键快捷键
 * 在弹窗打开、或焦点在输入框里的时候也会收到按键。`react-hotkeys-hook` 默认
 * 已经跳过 input/textarea/select，这里再补两种：
 *
 * 1. 焦点在弹窗里（沿用 `CommandBarDialog.tsx:166` 已有的
 *    `closest('[role="dialog"]')` 写法）；
 * 2. 页面上**存在**打开着的弹窗——有些弹窗（命令面板、确认框）会把焦点放在
 *    自己的输入框上，但也有把焦点留在 body 的，只看 activeElement 会漏。
 *
 * 判定一律「宁可漏触发，不可误触发」：拿不准就返回 true。
 */
export function isShortcutSuppressed(): boolean {
  if (typeof document === 'undefined') return false;

  if (document.querySelector('[role="dialog"]')) {
    return true;
  }

  const active = document.activeElement as HTMLElement | null;
  if (!active) return false;
  if (active.isContentEditable) return true;

  const tag = active.tagName;
  if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return true;

  return Boolean(active.closest('[role="dialog"]'));
}
