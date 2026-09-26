#!/usr/bin/env node
/**
 * 流水线技能包里**外来技能**的同步与校验。
 *
 *   node scripts/sync-pipeline-skills.mjs            # 等同 --check
 *   node scripts/sync-pipeline-skills.mjs --check    # 只校验，接进 pnpm run check 与 CI
 *   node scripts/sync-pipeline-skills.mjs --update   # 从来源拷贝并回写 sha256
 *
 * 锁文件的 `files` 是白名单：插件里该技能目录下的文件集合必须与它**完全相等**，
 * 多一个、少一个、内容差一个字节都算失败。平台自写的 vk-* 技能不在锁文件里。
 */
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const PLUGIN_DIR = path.join(ROOT, 'assets/pipeline-plugin');
const LOCK_PATH = path.join(PLUGIN_DIR, 'skills.lock.json');
const SKILLS_DIR = path.join(PLUGIN_DIR, 'skills');

const problems = [];
const fail = (message) => problems.push(message);

function expandHome(p) {
  return p.startsWith('~/') ? path.join(os.homedir(), p.slice(2)) : p;
}

function sha256(file) {
  return createHash('sha256').update(fs.readFileSync(file)).digest('hex');
}

function readLock() {
  const lock = JSON.parse(fs.readFileSync(LOCK_PATH, 'utf8'));
  if (lock.version !== 1) {
    throw new Error(`skills.lock.json 的 version 只支持 1，实际 ${lock.version}`);
  }
  return lock;
}

/** 来源目录；`kind: git` 时顺便核对本机检出的 HEAD 与锁文件一致。 */
function sourceDir(lock, skill, { verifyCommit }) {
  const source = lock.sources[skill.source];
  if (!source) throw new Error(`技能 ${skill.name} 的来源 ${skill.source} 不在 sources 里`);
  const local = expandHome(source.local);
  if (!fs.existsSync(local)) {
    throw new Error(`来源 ${skill.source} 的本机路径不存在：${local}`);
  }
  if (verifyCommit && source.kind === 'git') {
    const head = execFileSync('git', ['-C', local, 'rev-parse', 'HEAD'], {
      encoding: 'utf8',
    }).trim();
    if (head !== source.commit) {
      throw new Error(
        `来源 ${skill.source} 的本机检出是 ${head}，锁文件写的是 ${source.commit}；` +
          `先把本机检出切到锁定提交，或改锁文件后重跑 --update`
      );
    }
  }
  return path.join(local, skill.source_dir);
}

function listFiles(dir) {
  if (!fs.existsSync(dir)) return [];
  const out = [];
  const walk = (current, prefix) => {
    for (const entry of fs.readdirSync(current, { withFileTypes: true }).sort((a, b) =>
      a.name.localeCompare(b.name)
    )) {
      const rel = prefix ? `${prefix}/${entry.name}` : entry.name;
      if (entry.isDirectory()) walk(path.join(current, entry.name), rel);
      else out.push(rel);
    }
  };
  walk(dir, '');
  return out;
}

function check(lock) {
  for (const skill of lock.skills) {
    const dir = path.join(SKILLS_DIR, skill.name);
    const expected = Object.keys(skill.files).sort();
    const actual = listFiles(dir);
    for (const extra of actual.filter((f) => !expected.includes(f))) {
      fail(`skills/${skill.name}/${extra} 不在锁文件白名单里（多出来的文件请删掉或加进锁文件）`);
    }
    for (const name of expected) {
      const file = path.join(dir, name);
      if (!fs.existsSync(file)) {
        fail(`skills/${skill.name}/${name} 缺失，请跑 --update`);
        continue;
      }
      const want = skill.files[name];
      if (!want) {
        fail(`锁文件里 skills/${skill.name}/${name} 的哈希为空，请跑 --update`);
        continue;
      }
      const got = sha256(file);
      if (got !== want) {
        fail(
          `skills/${skill.name}/${name} 被改动：锁文件 ${want.slice(0, 12)}…，实际 ${got.slice(0, 12)}…；` +
            `外来技能不得手改，要改请提到上游再更新锁文件`
        );
      }
    }
  }
}

function update(lock) {
  for (const skill of lock.skills) {
    const from = sourceDir(lock, skill, { verifyCommit: true });
    const to = path.join(SKILLS_DIR, skill.name);
    fs.rmSync(to, { recursive: true, force: true });
    fs.mkdirSync(to, { recursive: true });
    const files = {};
    for (const name of Object.keys(skill.files).sort()) {
      const src = path.join(from, name);
      if (!fs.existsSync(src)) {
        throw new Error(`来源缺文件：${src}`);
      }
      const dest = path.join(to, name);
      fs.mkdirSync(path.dirname(dest), { recursive: true });
      fs.copyFileSync(src, dest);
      files[name] = sha256(dest);
    }
    skill.files = files;
    console.log(`同步 ${skill.name}（${Object.keys(files).length} 个文件）`);
  }
  fs.writeFileSync(LOCK_PATH, `${JSON.stringify(lock, null, 2)}\n`);
  console.log(`已回写 ${path.relative(ROOT, LOCK_PATH)}`);
}

const mode = process.argv.includes('--update') ? 'update' : 'check';
const lock = readLock();
if (mode === 'update') {
  update(lock);
  check(lock);
} else {
  check(lock);
}
if (problems.length > 0) {
  console.error('技能锁校验失败：');
  for (const p of problems) console.error(`  - ${p}`);
  console.error('修复办法：node scripts/sync-pipeline-skills.mjs --update');
  process.exit(1);
}
console.log('技能锁校验通过');
