import { describe, expect, it } from 'vitest';
import {
  isLoginFormValid,
  resolvePostLoginTarget,
  safeRedirect,
  validateLoginForm,
} from '@/features/local-auth/model/loginForm';

describe('登录表单校验', () => {
  it('用户名为空时报 usernameRequired', () => {
    expect(validateLoginForm({ username: '', password: 'x' })).toEqual({
      username: 'errors.usernameRequired',
    });
  });

  it('用户名全空白也算空', () => {
    expect(validateLoginForm({ username: '   ', password: 'x' })).toEqual({
      username: 'errors.usernameRequired',
    });
  });

  it('密码为空时报 passwordRequired', () => {
    expect(validateLoginForm({ username: 'a', password: '' })).toEqual({
      password: 'errors.passwordRequired',
    });
  });

  it('合法输入返回空错误对象', () => {
    const errors = validateLoginForm({ username: 'a', password: 'b' });
    expect(errors).toEqual({});
    expect(isLoginFormValid(errors)).toBe(true);
  });

  it('有错时 isLoginFormValid 为假', () => {
    expect(isLoginFormValid({ username: 'errors.usernameRequired' })).toBe(
      false
    );
  });
});

describe('开放重定向防护', () => {
  it('同源相对路径放行', () => {
    expect(safeRedirect('/projects/1')).toBe('/projects/1');
    expect(safeRedirect('/projects/1?tab=kanban')).toBe(
      '/projects/1?tab=kanban'
    );
  });

  it('协议相对地址被拒', () => {
    expect(safeRedirect('//evil.example')).toBe('/');
  });

  it('绝对 URL 被拒', () => {
    expect(safeRedirect('http://evil.example')).toBe('/');
    expect(safeRedirect('https://evil.example')).toBe('/');
    expect(safeRedirect('javascript:alert(1)')).toBe('/');
  });

  it('反斜杠变体被拒', () => {
    expect(safeRedirect('/\\evil')).toBe('/');
  });

  it('控制字符被拒', () => {
    expect(safeRedirect('/ok\nSet-Cookie: x=1')).toBe('/');
  });

  it('空值被拒', () => {
    expect(safeRedirect(null)).toBe('/');
    expect(safeRedirect(undefined)).toBe('/');
    expect(safeRedirect('')).toBe('/');
    expect(safeRedirect('   ')).toBe('/');
  });
});

describe('登录后的跳转目标', () => {
  it('没有 redirect 参数时回到当前路径', () => {
    expect(resolvePostLoginTarget('', '/projects/1')).toBe('/projects/1');
  });

  it('有 redirect 参数时用它', () => {
    expect(resolvePostLoginTarget('?redirect=%2Fworkspaces', '/login')).toBe(
      '/workspaces'
    );
  });

  it('redirect 参数同样受开放重定向防护', () => {
    expect(
      resolvePostLoginTarget('?redirect=https%3A%2F%2Fevil.example', '/login')
    ).toBe('/');
  });
});
