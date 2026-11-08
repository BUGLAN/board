/* ==========================================================================
   发版：npm run release -- <x.y.z> [--dry-run] [--push]
   版本号规则见 AGENTS.md「发版」：x 只由人决定；y 是重构或大升级，要先问人；z 是修复和比较小的新功能。

   1. 核对：版本号格式；工作区干净；HEAD 就是 origin/main；tag 还不存在；比上一个 tag 大
   2. 改版本号：Cargo.toml（workspace.package.version）和 Cargo.lock、package.json 和 web/package.json
      （连同各自的 package-lock.json）、src-tauri/tauri.conf.json
   3. 从上一个 tag 到现在的每个提交生成这一版的 changelog，写到 CHANGELOG.md 最上面
   4. 提交「chore: 发版 vX.Y.Z」，打附注 tag vX.Y.Z
   5. --push：推 main，再推 tag（推 tag 触发部署，见 docs/deploy.md「自动部署」）；不带时只打印推送命令

   --dry-run：只打印这一版的 changelog（有哪些提交），不改文件、不提交、不打 tag
   ========================================================================== */
import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const BRANCH = 'main';
const args = process.argv.slice(2);
const dry = args.includes('--dry-run');
const push = args.includes('--push');
const version = args.find(a => !a.startsWith('--'));

function run(cmd, argv) {
  return execFileSync(cmd, argv, { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
}
const git = (...argv) => run('git', argv);
function tryGit(...argv) {
  try {
    return git(...argv);
  } catch {
    return null;
  }
}
function fail(msg) {
  console.error(`发版中止：${msg}`);
  process.exit(1);
}

if (!version || !/^\d+\.\d+\.\d+$/.test(version)) fail('用法：npm run release -- <x.y.z> [--dry-run] [--push]');
const tag = `v${version}`;

/* ---------- 1. 核对 ---------- */

// 先拿到远端的 tag；--dry-run 连不上远端时用本地的
try {
  git('fetch', 'origin', '--tags');
} catch (e) {
  if (!dry) fail(`git fetch 失败：${String(e.stderr || e.message).trim()}`);
}

// 上一个 tag：HEAD 往前最近的 vX.Y.Z；没有就是第一次发版，changelog 从第一个提交算起
const prev = tryGit('describe', '--tags', '--abbrev=0', '--match', 'v[0-9]*', 'HEAD');
const parse = v => v.replace(/^v/, '').split('.').map(Number);
if (prev) {
  const [a, b] = [parse(prev), parse(tag)];
  const greater = b[0] !== a[0] ? b[0] > a[0] : b[1] !== a[1] ? b[1] > a[1] : b[2] > a[2];
  if (!greater) fail(`${tag} 要比上一个 tag ${prev} 大`);
}

if (!dry) {
  if (git('status', '--porcelain')) fail('工作区有没提交的改动');
  if (git('rev-parse', 'HEAD') !== git('rev-parse', `origin/${BRANCH}`))
    fail(`HEAD 不是 origin/${BRANCH}：先把要发的提交推到 ${BRANCH}，再在 origin/${BRANCH} 上发版`);
  if (tryGit('rev-parse', '-q', '--verify', `refs/tags/${tag}`) || git('ls-remote', '--tags', 'origin', `refs/tags/${tag}`))
    fail(`tag ${tag} 已经存在`);
}

/* ---------- 3. changelog（先生成：没有新提交就不用改版本号） ---------- */

const GROUPS = [
  ['feat', '新功能'],
  ['fix', '修复'],
  ['perf', '性能'],
  ['refactor', '重构'],
  ['docs', '文档'],
];
const OTHER = '其他';

function today() {
  const d = new Date();
  const p = n => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

function section() {
  const range = prev ? `${prev}..HEAD` : 'HEAD';
  const raw = git('log', '--no-merges', '--format=%h%x1f%s%x1f%b%x1e', range);
  const groups = new Map([...GROUPS.map(([, t]) => [t, []]), [OTHER, []]]);
  for (const rec of raw.split('\x1e')) {
    if (!rec.trim()) continue;
    const [sha, subject, body = ''] = rec.trim().split('\x1f');
    if (/^chore: 发版 v\d/.test(subject)) continue; // 上一次发版的提交
    const m = subject.match(/^(\w+)(?:\(([^)]+)\))?(!)?:\s*(.+)$/);
    const type = m ? m[1] : '';
    const scope = m?.[2];
    const desc = m ? m[4] : subject;
    const breaking = Boolean(m?.[3]) || /^BREAKING[ -]CHANGE:/m.test(body);
    const card = body.match(/^卡片：(\S+)\s*$/m)?.[1];
    const title = GROUPS.find(([t]) => t === type)?.[1] ?? OTHER;
    const note = [sha, card && `卡片 ${card}`].filter(Boolean).join(' · ');
    groups.get(title).push(`- ${breaking ? '**不兼容** ' : ''}${scope ? `**${scope}**：` : ''}${desc}（${note}）`);
  }
  const parts = [...groups].filter(([, lines]) => lines.length).map(([t, lines]) => `### ${t}\n\n${lines.join('\n')}`);
  if (!parts.length) fail(prev ? `${prev} 之后没有新提交` : '没有提交');
  return `## ${tag} · ${today()}\n\n${parts.join('\n\n')}\n`;
}

const notes = section();
if (dry) {
  console.log(`上一个 tag：${prev ?? '（没有，第一次发版）'}\n`);
  console.log(notes);
  process.exit(0);
}

/* ---------- 2. 改版本号 ---------- */

function edit(file, fn) {
  const path = join(root, file);
  const before = readFileSync(path, 'utf8');
  const after = fn(before);
  if (after === before) fail(`${file} 里没找到要改的版本号`);
  writeFileSync(path, after);
}

// Cargo.toml：只改 [workspace.package] 这一节的 version
edit('Cargo.toml', s => {
  const start = s.indexOf('[workspace.package]');
  if (start < 0) return s;
  const next = s.indexOf('\n[', start + 1);
  const end = next < 0 ? s.length : next;
  const body = s.slice(start, end).replace(/^(version\s*=\s*")[^"]*(")/m, `$1${version}$2`);
  return s.slice(0, start) + body + s.slice(end);
});
// Cargo.lock 里本仓库 crate 的版本号，否则 CI 的 cargo test --locked 不过
run('cargo', ['update', '--workspace']);
run('cargo', ['metadata', '--locked', '--no-deps', '--format-version', '1']);
// package.json 和 package-lock.json（只改版本号，不提交、不打 tag）
for (const dir of ['.', 'web']) {
  execFileSync('npm', ['version', version, '--no-git-tag-version', '--allow-same-version'], {
    cwd: join(root, dir),
    stdio: ['ignore', 'ignore', 'inherit'],
  });
}
edit('src-tauri/tauri.conf.json', s => s.replace(/^(\s*"version":\s*")[^"]*(")/m, `$1${version}$2`));

/* ---------- 3. 写 CHANGELOG.md ---------- */

const HEADER = '# 更新日志\n\n版本号规则见 [AGENTS.md](AGENTS.md)「发版」。每一版由 `npm run release` 从上一版之后的提交生成。\n';
const file = join(root, 'CHANGELOG.md');
const old = existsSync(file) ? readFileSync(file, 'utf8') : HEADER;
const at = old.indexOf('\n## ');
writeFileSync(file, at < 0 ? `${old.trimEnd()}\n\n${notes}` : `${old.slice(0, at + 1)}${notes}\n${old.slice(at + 1)}`);

/* ---------- 4. 提交、打 tag ---------- */

git(
  'add',
  'Cargo.toml',
  'Cargo.lock',
  'package.json',
  'package-lock.json',
  'web/package.json',
  'web/package-lock.json',
  'src-tauri/tauri.conf.json',
  'CHANGELOG.md',
);
git('commit', '-m', `chore: 发版 ${tag}`, '-m', `版本号改成 ${version}，CHANGELOG.md 加上 ${tag} 一节。`);
git('tag', '-a', tag, '-m', `Board ${tag}`);
console.log(notes);
console.log(`已提交「chore: 发版 ${tag}」并打了 tag ${tag}（上一个 tag：${prev ?? '没有'}）。`);

/* ---------- 5. 推送 ---------- */

const cmds = [`git push origin HEAD:${BRANCH}`, `git push origin ${tag}`];
if (!push) {
  console.log(`\n确认无误后推送（推 tag 会部署线上服务端）：\n  ${cmds.join('\n  ')}`);
  process.exit(0);
}
try {
  git('push', 'origin', `HEAD:${BRANCH}`);
} catch (e) {
  console.error(String(e.stderr || e.message).trim());
  fail(`${BRANCH} 推送被拒（别人先推了）。撤销这次发版后重来：git tag -d ${tag} && git reset --hard HEAD~1`);
}
git('push', 'origin', tag);
console.log(`已推送 ${BRANCH} 和 ${tag}，部署见 GitHub Actions → deploy。`);
