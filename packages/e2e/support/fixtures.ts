import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { test as base, expect } from '@playwright/test';
import { VkApi } from './api';
import { REPOS_DIR } from './env';

export interface E2eProject {
  projectId: string;
  projectName: string;
  repoId: string;
  repoName: string;
}

/**
 * 在临时目录建一个有一次提交的 git 仓库（main 分支）。
 *
 * qa-mode 只把「扫描本机仓库」换成两个固定的 GitHub QA 仓库
 * （`crates/services/src/services/filesystem.rs:101-105`），`POST /api/repos`
 * 注册任意路径不受影响，所以这里注册本地临时仓库，不触发任何外网 clone。
 */
export function createGitRepo(dir: string): string {
  fs.mkdirSync(dir, { recursive: true });
  const git = (...args: string[]) =>
    execFileSync('git', args, { cwd: dir, stdio: 'pipe' });
  git('init', '-b', 'main');
  fs.writeFileSync(path.join(dir, 'README.md'), '# vk e2e fixture\n');
  git('add', '.');
  git(
    '-c',
    'user.name=vk-e2e',
    '-c',
    'user.email=vk-e2e@example.com',
    'commit',
    '-m',
    'init'
  );
  return dir;
}

let counter = 0;
function uniqueSlug(): string {
  counter += 1;
  return `${Date.now().toString(36)}-${counter}`;
}

export async function createProjectWithRepo(
  api: VkApi,
  projectName: string
): Promise<E2eProject> {
  await api.prepareConfig();
  const repoName = `e2e-repo-${uniqueSlug()}`;
  const repo = await api.registerRepo(
    createGitRepo(path.join(REPOS_DIR, repoName)),
    repoName
  );
  const project = await api.createProject(projectName);
  return { projectId: project.id, projectName, repoId: repo.id, repoName };
}

export const test = base.extend<{ api: VkApi; project: E2eProject }>({
  // eslint 不覆盖本包；Playwright 要求第一个参数是对象解构
  api: async ({}, use) => {
    await use(new VkApi());
  },
  project: async ({ api }, use, testInfo) => {
    await use(
      await createProjectWithRepo(api, `E2E ${testInfo.title}`.slice(0, 60))
    );
  },
});

export { expect };
