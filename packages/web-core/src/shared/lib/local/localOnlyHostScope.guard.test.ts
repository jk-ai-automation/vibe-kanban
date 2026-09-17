import { readdirSync, readFileSync, statSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/**
 * 「本机专属接口不许被转发到远端 host」的**源码级**护栏。
 *
 * 背景：`makeLocalApiRequest` 默认 `hostScope: 'current'`，选中远端 host 时
 * 会把 `/api/xxx` 改写成 `/api/host/<id>/xxx` 转发过去。本地认证与管理接口
 * 只存在于本机、认的是本机会话 Cookie，转过去必然 401，用户当场被踢回登录页。
 *
 * 防线是**两层**，这个文件两层都盯：
 *
 * 1. **强制点**：`localApiTransport.ts` 的 `LOCAL_ONLY_API_PREFIXES`。
 *    不管调用方怎么写都挡得住，所以「三个前缀在不在名单里」是本文件
 *    **最要紧**的一条断言——它被删掉必须变红。
 * 2. **双保险兼文档**：调用点上的 `hostScope: 'none'`。少写一处不会真出事
 *    （第 1 层兜着），但读代码的人会看不出这条是本机专属，所以这里仍然
 *    要求每一处都标。这一条报红时是**提醒补标注**，不是「线上要坏了」。
 *
 * 配套的 `localOnlyHostScope.test.ts` 从行为侧验证同样两层。
 */

const 仓库根 = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  '../../../../../..'
);

/** 这些前缀下的接口只存在于本机后端。 */
const 本机专属前缀 = ['/api/local-auth/', '/api/admin/'];

/** 会被 host 改写的那两个入口。 */
const 传输层调用 = /\b(?:makeLocalApiRequest|openLocalApiWebSocket)\s*\(/g;

/**
 * 「这一处已经标成本机了」的两种写法：直接写死，或者展开本文件里那个
 * `const 本机 = { hostScope: 'none' }`（`bootstrapApi.ts` 用的就是后者）。
 */
const 本机标记 = /hostScope:\s*'none'|\.\.\.本机|,\s*本机\)/g;

/**
 * 定义本机专属路径表的文件，**全仓就这三个**。
 *
 * 新开第四个，说明多了一组只存在于本机的接口——那就得同时想清楚它怎么
 * 保证不被转发，并把它加进 `localOnlyHostScope.test.ts` 的遍历名单。
 * 所以这里故意钉死，逼人回来看这段注释。
 */
const 路径表文件 = [
  'packages/web-core/src/shared/lib/local/adminApi.ts',
  'packages/web-core/src/shared/lib/local/bootstrapApi.ts',
  'packages/web-core/src/shared/lib/local/workspaceDeleteRequestsApi.ts',
];

const 路径表名 = [
  'LOCAL_AUTH_PATHS',
  'ADMIN_API_PATHS',
  'DELETE_REQUEST_API_PATHS',
];

interface 源文件 {
  相对路径: string;
  正文: string;
}

function 遍历(目录: string, 收集: (文件: string) => void): void {
  for (const 条目 of readdirSync(目录)) {
    if (条目 === 'node_modules' || 条目 === 'dist' || 条目.startsWith('.')) {
      continue;
    }
    const 全路径 = path.join(目录, 条目);
    if (statSync(全路径).isDirectory()) {
      遍历(全路径, 收集);
    } else if (/\.tsx?$/.test(条目) && !/\.test\.tsx?$/.test(条目)) {
      收集(全路径);
    }
  }
}

/** 注释里提到 `/api/admin/...` 的地方太多了，扫之前先把注释去掉。 */
function 去注释(正文: string): string {
  return 正文.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');
}

function 计数(正文: string, 模式: RegExp): number {
  return (正文.match(new RegExp(模式.source, 'g')) ?? []).length;
}

const 全部源文件: 源文件[] = (() => {
  const 结果: 源文件[] = [];
  for (const 包 of readdirSync(path.join(仓库根, 'packages'))) {
    const src = path.join(仓库根, 'packages', 包, 'src');
    try {
      if (!statSync(src).isDirectory()) continue;
    } catch {
      continue;
    }
    遍历(src, (全路径) => {
      结果.push({
        相对路径: path.relative(仓库根, 全路径),
        正文: 去注释(readFileSync(全路径, 'utf8')),
      });
    });
  }
  return 结果;
})();

function 碰了本机接口(正文: string): boolean {
  return (
    本机专属前缀.some((前缀) => 正文.includes(前缀)) ||
    路径表名.some((名) => 正文.includes(名))
  );
}

describe('本机专属接口的源码护栏', () => {
  /**
   * **这是真正的强制点**：名单里少一条，对应那组接口在选中远端 host 时
   * 就会被转发出去、401、把用户踢回登录页。调用点的标注只是第二层，
   * 挡不住「新加的文件忘了标」。
   */
  it('三个本机专属前缀都在 LOCAL_ONLY_API_PREFIXES 里', () => {
    const 传输层 = readFileSync(
      path.join(
        仓库根,
        'packages/web-core/src/shared/lib/localApiTransport.ts'
      ),
      'utf8'
    );
    const 名单 = 传输层.match(
      /const LOCAL_ONLY_API_PREFIXES\s*=\s*\[([\s\S]*?)\]/
    )?.[1];

    expect(
      名单,
      '没在 localApiTransport.ts 里找到 LOCAL_ONLY_API_PREFIXES'
    ).toBeTruthy();
    for (const 前缀 of [
      '/api/local-auth',
      '/api/admin',
      '/api/workspace-delete-requests',
    ]) {
      expect(名单, `${前缀} 不在中央名单里`).toContain(`'${前缀}'`);
    }
  });

  it('扫到了源文件（别让护栏悄悄空跑）', () => {
    expect(全部源文件.length).toBeGreaterThan(100);
    expect(
      全部源文件.filter((文件) => 碰了本机接口(文件.正文)).length
    ).toBeGreaterThan(0);
  });

  it('碰本机接口的文件，每一处传输层调用都标成了本机', () => {
    const 漏标: string[] = [];

    for (const { 相对路径, 正文 } of 全部源文件) {
      if (!碰了本机接口(正文)) continue;

      const 调用数 = 计数(正文, 传输层调用);
      if (调用数 === 0) continue; // 不直接发请求（只放常量、或走 requestLocalEnvelope）

      // `const 本机 = { hostScope: 'none' }` 这句定义本身不算一处标记，
      // 否则文件里少标一处也凑得够数。
      const 标记数 = 计数(正文.replace(/const\s+本机[^;]*;/g, ''), 本机标记);
      if (标记数 < 调用数) {
        漏标.push(`${相对路径}：${调用数} 处调用，只有 ${标记数} 处标了本机`);
      }
    }

    expect(
      漏标,
      "这些文件请求本机专属接口时漏了 hostScope: 'none'。中央名单" +
        '（LOCAL_ONLY_API_PREFIXES）已经兜住了，线上不会坏；但请补上标注，' +
        '否则读代码的人看不出这条是本机专属'
    ).toEqual([]);
  });

  it('没人绕开传输层直接 fetch 本机专属接口', () => {
    const 越界: string[] = [];
    // fetch('/api/admin/...') / fetch(`/api/local-auth/...`)
    const 裸调用 = /fetch\s*\(\s*['"`]\/api\/(?:local-auth|admin)\//g;

    for (const { 相对路径, 正文 } of 全部源文件) {
      if (相对路径.includes('localApiTransport')) continue; // 传输层自己
      if (裸调用.test(正文)) 越界.push(相对路径);
      裸调用.lastIndex = 0;
    }

    expect(
      越界,
      '本机专属接口必须走 makeLocalApiRequest（CSRF 头、会话过期广播都在那里）'
    ).toEqual([]);
  });

  it('requestLocalEnvelope 自己把 hostScope 钉死成 none', () => {
    const 正文 = 去注释(
      readFileSync(
        path.join(仓库根, 'packages/web-core/src/shared/lib/local/adminApi.ts'),
        'utf8'
      )
    );

    expect(正文).toMatch(
      /export async function requestLocalEnvelope[\s\S]*?hostScope:\s*'none'[\s\S]*?makeLocalApiRequest/
    );
  });

  it('本机专属路径表就这三张，新增一张必须回来更新两条测试', () => {
    const 定义路径表 = 全部源文件
      .filter(({ 正文 }) =>
        路径表名.some((名) => 正文.includes(`export const ${名}`))
      )
      .map(({ 相对路径 }) => 相对路径.split(path.sep).join('/'))
      .sort();

    expect(定义路径表).toEqual(路径表文件);
  });
});
