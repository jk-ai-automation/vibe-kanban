import { afterEach, describe, expect, it, vi } from 'vitest';

import { artifactHtmlReportUrl } from './pipelineApi';
import {
  canLinkLocalApiDirectly,
  scopedLocalApiPath,
  setLocalApiTransport,
} from '@/shared/lib/localApiTransport';

/**
 * 「查看完整报告」是普通链接，不经 `makeLocalApiRequest`，
 * 所以要自己按当前主机改写路径；远程网页端（WebRTC 传输）普通链接到不了后端。
 */
const host = vi.hoisted(() => ({ id: null as string | null }));

vi.mock('@/shared/providers/HostIdProvider', () => ({
  getCurrentHostId: () => host.id,
}));

afterEach(() => {
  host.id = null;
  setLocalApiTransport(null);
});

const 非默认传输 = {
  request: () => Promise.resolve(new Response(null)),
  openWebSocket: () => {
    throw new Error('not used');
  },
};

describe('scopedLocalApiPath', () => {
  it('没选远端主机时原样返回', () => {
    expect(scopedLocalApiPath('/api/local/pipeline/artifacts/a1')).toBe(
      '/api/local/pipeline/artifacts/a1'
    );
  });

  it('选中远端主机时改写成 /api/host/<id>/...', () => {
    host.id = 'h1';
    expect(scopedLocalApiPath('/api/local/pipeline/artifacts/a1')).toBe(
      '/api/host/h1/local/pipeline/artifacts/a1'
    );
  });

  it('本机专属接口不改写', () => {
    host.id = 'h1';
    expect(scopedLocalApiPath('/api/admin/users')).toBe('/api/admin/users');
  });
});

describe('canLinkLocalApiDirectly', () => {
  it('默认 HTTP 传输时可以直接链接', () => {
    expect(canLinkLocalApiDirectly()).toBe(true);
  });

  it('非默认传输（远程网页端）时不能直接链接', () => {
    setLocalApiTransport(非默认传输);
    expect(canLinkLocalApiDirectly()).toBe(false);
  });
});

describe('artifactHtmlReportUrl', () => {
  it('选中远端主机时链接按主机改写', () => {
    host.id = 'h1';
    expect(artifactHtmlReportUrl('a1')).toBe(
      '/api/host/h1/local/pipeline/artifacts/a1/html-report'
    );
  });

  it('本机时就是契约路径', () => {
    expect(artifactHtmlReportUrl('a1')).toBe(
      '/api/local/pipeline/artifacts/a1/html-report'
    );
  });

  it('非默认传输时返回 null（界面不显示链接）', () => {
    setLocalApiTransport(非默认传输);
    expect(artifactHtmlReportUrl('a1')).toBeNull();
  });
});
