import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));

/** 仓库根目录（packages/e2e/support → 上三级）。 */
export const REPO_ROOT = path.resolve(here, '../../..');

export const BACKEND_PORT = Number(process.env.VK_E2E_BACKEND_PORT ?? 4310);
export const FRONTEND_PORT = Number(process.env.VK_E2E_FRONTEND_PORT ?? 4311);

/**
 * 本次运行的临时根目录。
 *
 * 每次 `playwright test` 新建一个（`vk-e2e-XXXXXX`），并写回
 * `process.env.VK_E2E_ROOT`：配置文件先在主进程里加载，之后启动的 worker
 * 继承这个环境变量，拿到的是同一个目录。显式设置 `VK_E2E_ROOT` 时沿用它。
 */
function resolveE2eRoot(): string {
  const preset = process.env.VK_E2E_ROOT;
  if (preset) return preset;
  const created = fs.mkdtempSync(path.join(os.tmpdir(), 'vk-e2e-'));
  process.env.VK_E2E_ROOT = created;
  return created;
}

export const E2E_ROOT = resolveE2eRoot();
/** 后端数据目录（数据库、配置）：`VK_ASSET_DIR`。 */
export const ASSET_DIR = path.join(E2E_ROOT, 'assets');
/** fixtures 建的临时 git 仓库放这里。 */
export const REPOS_DIR = path.join(E2E_ROOT, 'repos');

export const BACKEND_URL = `http://127.0.0.1:${BACKEND_PORT}`;
/** 必须与 `VK_ALLOWED_ORIGINS` 逐字一致（`crates/server/src/middleware/origin.rs`）。 */
export const FRONTEND_URL = `http://localhost:${FRONTEND_PORT}`;
