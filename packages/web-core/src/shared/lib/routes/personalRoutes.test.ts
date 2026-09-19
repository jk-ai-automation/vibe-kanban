import { describe, expect, it } from 'vitest';
import { PERSONAL_ROUTES, personalNavActiveKey } from './personalRoutes';

describe('PERSONAL_ROUTES', () => {
  it('固定路径与带参路径', () => {
    expect(PERSONAL_ROUTES.workbench).toBe('/home');
    expect(PERSONAL_ROUTES.docs).toBe('/docs');
    expect(PERSONAL_ROUTES.testing).toBe('/testing');
    expect(PERSONAL_ROUTES.project('p1')).toBe('/projects/p1');
    expect(PERSONAL_ROUTES.issueDetail('p1', 'i 1')).toBe(
      '/projects/p1/issues/i%201/detail'
    );
  });
});

describe('personalNavActiveKey', () => {
  it('按路径前缀判断当前导航项', () => {
    expect(personalNavActiveKey('/home')).toBe('workbench');
    expect(personalNavActiveKey('/projects/p1')).toBe('pipeline');
    expect(personalNavActiveKey('/projects/p1/issues/i1/detail')).toBe(
      'pipeline'
    );
    expect(personalNavActiveKey('/testing')).toBe('testing');
    expect(personalNavActiveKey('/docs')).toBe('docs');
  });

  it('其它页面没有高亮项；前缀不能误伤', () => {
    expect(personalNavActiveKey('/workspaces')).toBeNull();
    expect(personalNavActiveKey('/homepage')).toBeNull();
    expect(personalNavActiveKey('/')).toBeNull();
  });
});
