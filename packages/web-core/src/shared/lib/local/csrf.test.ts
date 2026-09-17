import { describe, expect, it } from 'vitest';
import {
  methodNeedsCsrf,
  readCsrfToken,
  readCsrfTokenFrom,
} from '@/shared/lib/local/csrf';

describe('读取 vk_csrf Cookie', () => {
  it('读到 vk_csrf', () => {
    expect(readCsrfTokenFrom('a=1; vk_csrf=xyz; b=2')).toBe('xyz');
  });

  it('只有 vk_csrf 一个 Cookie 时也能读到', () => {
    expect(readCsrfTokenFrom('vk_csrf=xyz')).toBe('xyz');
  });

  it('没有时返回 null', () => {
    expect(readCsrfTokenFrom('a=1; b=2')).toBeNull();
  });

  it('不前缀匹配', () => {
    expect(readCsrfTokenFrom('vk_csrf_x=bad')).toBeNull();
  });

  it('不后缀匹配', () => {
    expect(readCsrfTokenFrom('xvk_csrf=bad')).toBeNull();
  });

  it('空值返回 null', () => {
    expect(readCsrfTokenFrom('vk_csrf=')).toBeNull();
    expect(readCsrfTokenFrom('a=1; vk_csrf=  ; b=2')).toBeNull();
  });

  it('大小写敏感', () => {
    expect(readCsrfTokenFrom('VK_CSRF=x')).toBeNull();
  });

  it('重复时取第一个', () => {
    expect(readCsrfTokenFrom('vk_csrf=a; vk_csrf=b')).toBe('a');
  });

  it('不做 decodeURIComponent（服务端令牌是 base64url，无需编码）', () => {
    expect(readCsrfTokenFrom('vk_csrf=a%2Bb')).toBe('a%2Bb');
  });

  it('空字符串与 null 输入返回 null', () => {
    expect(readCsrfTokenFrom('')).toBeNull();
    expect(readCsrfTokenFrom(null)).toBeNull();
  });

  it('没有 document 时 readCsrfToken 返回 null 而不是抛错', () => {
    expect(typeof document).toBe('undefined');
    expect(readCsrfToken()).toBeNull();
  });
});

describe('哪些方法需要 CSRF 头', () => {
  it('安全方法不需要', () => {
    for (const method of ['GET', 'get', 'HEAD', 'OPTIONS', undefined]) {
      expect(methodNeedsCsrf(method)).toBe(false);
    }
  });

  it('写方法需要', () => {
    for (const method of ['POST', 'put', 'PATCH', 'DELETE']) {
      expect(methodNeedsCsrf(method)).toBe(true);
    }
  });
});
