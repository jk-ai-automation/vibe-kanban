import { describe, expect, it } from 'vitest';
import {
  Action,
  Scope,
  getBindingFor,
  getKeysFor,
  keyBindings,
  SEQUENCE_FIRST_KEYS,
  sequentialBindings,
} from './registry';

describe('看板快捷键键位（设计文档 §7.5）', () => {
  it('新建同时认 c 与 n —— 不替换老键位，避免打断肌肉记忆', () => {
    const keys = getKeysFor(Action.CREATE, Scope.KANBAN);
    expect(keys).toContain('c');
    expect(keys).toContain('n');
  });

  it('项目页的新建仍然只有 c（没被 n 污染）', () => {
    expect(getKeysFor(Action.CREATE, Scope.PROJECTS)).toEqual(['c']);
  });

  it('裸 Enter 打开需求，和 meta+enter 的 OPEN_DETAILS 是两条独立绑定', () => {
    expect(getKeysFor(Action.OPEN_ISSUE, Scope.KANBAN)).toEqual(['enter']);
    expect(getKeysFor(Action.OPEN_DETAILS, Scope.KANBAN)).toEqual([
      'meta+enter',
      'ctrl+enter',
    ]);
  });

  it('裸 Enter 的 KANBAN 绑定不会和 DIALOG scope 的 SUBMIT 撞', () => {
    const submit = getBindingFor(Action.SUBMIT, Scope.DIALOG);
    expect(submit?.keys).toBe('enter');
    expect(submit?.scopes).toEqual([Scope.DIALOG]);

    const openIssue = getBindingFor(Action.OPEN_ISSUE, Scope.KANBAN);
    expect(openIssue?.scopes).toEqual([Scope.KANBAN]);
  });

  it('e 编辑已注册在 KANBAN scope', () => {
    expect(getKeysFor(Action.EDIT, Scope.KANBAN)).toEqual(['e']);
  });

  it('/ 搜索、j/k/h/l 导航仍在', () => {
    expect(getKeysFor(Action.FOCUS_SEARCH, Scope.KANBAN)).toEqual(['slash']);
    expect(getKeysFor(Action.NAV_DOWN, Scope.KANBAN)).toEqual(['j']);
    expect(getKeysFor(Action.NAV_UP, Scope.KANBAN)).toEqual(['k']);
    expect(getKeysFor(Action.NAV_LEFT, Scope.KANBAN)).toEqual(['h']);
    expect(getKeysFor(Action.NAV_RIGHT, Scope.KANBAN)).toEqual(['l']);
  });

  it('? 帮助仍绑在 shift+slash 的 GLOBAL scope', () => {
    expect(getKeysFor(Action.SHOW_HELP, Scope.GLOBAL)).toEqual(['shift+slash']);
  });

  it('Esc 在 KANBAN scope 只有一条绑定（分级处理在同一个 handler 里）', () => {
    const escInKanban = keyBindings.filter(
      (binding) =>
        binding.action === Action.EXIT &&
        binding.keys === 'esc' &&
        binding.scopes?.includes(Scope.KANBAN)
    );
    expect(escInKanban).toHaveLength(1);
  });
});

describe('单键与连击前缀的冲突面', () => {
  const singleKanbanKeys = keyBindings
    .filter((binding) => binding.scopes?.includes(Scope.KANBAN))
    .flatMap((binding) =>
      Array.isArray(binding.keys) ? binding.keys : [binding.keys]
    )
    .filter((key) => key.length === 1);

  it('看板单键里没有任何一个是连击的第一个键', () => {
    // 否则按下它会直接触发动作，连击永远没机会形成。
    for (const key of singleKanbanKeys) {
      expect(SEQUENCE_FIRST_KEYS.has(key)).toBe(false);
    }
  });

  it('c / n / h / l 确实是某些连击的第二个键，所以必须做连击中护栏', () => {
    const secondKeys = new Set(sequentialBindings.map((b) => b.keys[1]));
    for (const key of ['c', 'n', 'h', 'l']) {
      expect(secondKeys.has(key)).toBe(true);
    }
  });

  it('e 不是任何连击的第二个键', () => {
    const secondKeys = new Set(sequentialBindings.map((b) => b.keys[1]));
    expect(secondKeys.has('e')).toBe(false);
  });
});
