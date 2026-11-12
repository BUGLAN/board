/* ==========================================================================
   发版：npm run release -- <x.y.z> [--dry-run] [--push]
   版本号规则见 AGENTS.md「发版」：x 只由人决定；y 是重构或大升级，要先问人；z 是修复和比较小的新功能。

   1. 拉到最新：git fetch；工作区要干净；本地只是落后于 origin/main（别的电脑先推了）时自动快进过去，
      本地有没推到远端的提交时中止（先按 AGENTS.md 推到 main）。只在 main 分支或分离 HEAD 上发版
   2. 核对：版本号格式；tag 还不存在；比上一个 tag 大
   3. 从上一个 tag 到现在的每个提交生成这一版的 changelog（先在内存里生成，没有新提交就不用改版本号）
   4. 改版本号：Cargo.toml（workspace.package.version）和 Cargo.lock、package.json 和 web/package.json
      （连同各自的 package-lock.json）、src-tauri/tauri.conf.json
   5. 把第 3 步生成的这一节写到 CHANGELOG.md 最上面
   6. 提交「chore: 发版 vX.Y.Z」，打附注 tag vX.Y.Z（说明里是这一版的 changelog）
   7. --push：推 main，再推 tag（推 tag 触发部署，见 docs/deploy.md「自动部署」）；不带时只打印推送命令。
      推 main 被拒（发版途中别人推了 main）：撤掉这次的发版提交和 tag，重新拉到最新再发一次；第二次还被拒才停下

   --dry-run：只打印这一版的 changelog（按 origin/main 列，和真正发出去的一致），不改文件、不提交、不打 tag、不动工作区
   ========================================================================== */
import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const BRANCH = 'main';
// Windows 上 npm 是 npm.cmd，execFileSync 不认 PATHEXT，直接写 'npm' 会 ENOENT
const NPM = process.platform === 'win32' ? 'npm.cmd' : 'npm';
const args = process.argv.slice(2);
const dry = args.includes('--dry-run');
const push = args.includes('--push');
/** 推 main 被拒后自动重发的那一次（脚本自己加的参数） */
const retried = args.includes('--retried');
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

/* ---------- 1. 拉到最新 ---------- */

// 先拿到远端的提交和 tag；--dry-run 连不上远端时用本地的
try {
  git('fetch', 'origin', '--tags');
} catch (e) {
  if (!dry) fail(`git fetch 失败：${String(e.stderr || e.message).trim()}`);
}

const REMOTE = `origin/${BRANCH}`;
const hasRemote = Boolean(tryGit('rev-parse', '-q', '--verify', `refs/remotes/${REMOTE}`));
if (!dry) {
  if (!hasRemote) fail(`没有 ${REMOTE}：先 git remote add origin <仓库地址>`);
  if (git('status', '--porcelain')) fail('工作区有没提交的改动');
  const head = git('rev-parse', 'HEAD');
  const remote = git('rev-parse', REMOTE);
  if (head !== remote) {
    // 本地有远端没有的提交：发出去的会和集成分支不一样，先按正常流程推上去
    const ahead = git('log', '--oneline', `${REMOTE}..HEAD`);
    if (ahead)
      fail(
        `本地有没推到 ${BRANCH} 的提交，先按 AGENTS.md rebase 到 ${REMOTE}、跑完检查推上去，再发版：\n${ahead}`,
      );
    // 只是落后（别的电脑先推了）：快进到 origin/main
    const branch = tryGit('symbolic-ref', '-q', '--short', 'HEAD');
    if (branch && branch !== BRANCH) fail(`当前在分支 ${branch} 上：切到 ${BRANCH}（或分离 HEAD）再发版`);
    const n = git('rev-list', '--count', `HEAD..${REMOTE}`);
    if (branch) git('merge', '--ff-only', '-q', REMOTE);
    else git('checkout', '-q', '--detach', REMOTE);
    console.log(`本地落后 ${REMOTE} ${n} 个提交，已经快进到最新（${git('rev-parse', '--short', 'HEAD')}）。`);
  }
}

/* ---------- 2. 核对 ---------- */

// 要发的提交：发版时 HEAD 已经是 origin/main；--dry-run 不动工作区，直接看 origin/main
const TIP = dry && hasRemote ? REMOTE : 'HEAD';

// 上一个 tag：TIP 往前最近的 vX.Y.Z；没有就是第一次发版，changelog 从第一个提交算起
const prev = tryGit('describe', '--tags', '--abbrev=0', '--match', 'v[0-9]*', TIP);
const parse = v => v.replace(/^v/, '').split('.').map(Number);
if (prev) {
  const [a, b] = [parse(prev), parse(tag)];
  const greater = b[0] !== a[0] ? b[0] > a[0] : b[1] !== a[1] ? b[1] > a[1] : b[2] > a[2];
  if (!greater) fail(`${tag} 要比上一个 tag ${prev} 大`);
}

if (!dry) {
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
  const range = prev ? `${prev}..${TIP}` : TIP;
  const raw = git('log', '--no-merges', '--format=%h%x1f%s%x1f%b%x1e', range);
  const groups = new Map([...GROUPS.map(([, t]) => [t, []]), [OTHER, []]]);
  // 同一张卡片（提交正文最后的「卡片：…」）打回后会再提交，这一版里只留最新的一条。
  // git log 从新到旧，先遇到的就是最新的；更早的提交不兼容时，留下的那条也标「不兼容」
  const byCard = new Map();
  for (const rec of raw.split('\x1e')) {
    if (!rec.trim()) continue;
    const [sha, subject, body = ''] = rec.trim().split('\x1f');
    if (/^chore: 发版 v\d/.test(subject)) continue; // 上一次发版的提交
    const m = subject.match(/^(\w+)(?:\(([^)]+)\))?(!)?:\s*(.+)$/);
    const breaking = Boolean(m?.[3]) || /^BREAKING[ -]CHANGE:/m.test(body);
    const card = body.match(/^卡片：(\S+)\s*$/m)?.[1];
    const seen = card && byCard.get(card);
    if (seen) {
      seen.breaking ||= breaking;
      continue;
    }
    const entry = { sha, type: m ? m[1] : '', scope: m?.[2], desc: m ? m[4] : subject, breaking };
    if (card) byCard.set(card, entry);
    groups.get(GROUPS.find(([t]) => t === entry.type)?.[1] ?? OTHER).push(entry);
  }
  // 卡片号本身不写进来：changelog 和 GitHub Release 给使用者看，白板他们打不开
  const line = e => `- ${e.breaking ? '**不兼容** ' : ''}${e.scope ? `**${e.scope}**：` : ''}${e.desc}（${e.sha}）`;
  for (const [t, entries] of groups) groups.set(t, entries.map(line));
  const parts = [...groups].filter(([, lines]) => lines.length).map(([t, lines]) => `### ${t}\n\n${lines.join('\n')}`);
  if (!parts.length) fail(prev ? `${prev} 之后没有新提交` : '没有提交');
  return `## ${tag} · ${today()}\n\n${parts.join('\n\n')}\n`;
}

const notes = section();
if (dry) {
  const behind = TIP !== 'HEAD' ? Number(git('rev-list', '--count', `HEAD..${TIP}`)) : 0;
  if (behind) console.log(`本地落后 ${TIP} ${behind} 个提交：下面按 ${TIP} 列，真正发版时会先快进过去。\n`);
  console.log(`上一个 tag：${prev ?? '（没有，第一次发版）'}\n`);
  console.log(notes);
  process.exit(0);
}

/* ---------- 4. 改版本号 ---------- */

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
  execFileSync(NPM, ['version', version, '--no-git-tag-version', '--allow-same-version'], {
    cwd: join(root, dir),
    stdio: ['ignore', 'ignore', 'inherit'],
    // Node 22 起 Windows 上直接 spawn .cmd 会 EINVAL，必须经过 shell
    shell: process.platform === 'win32',
  });
}
edit('src-tauri/tauri.conf.json', s => s.replace(/^(\s*"version":\s*")[^"]*(")/m, `$1${version}$2`));

/* ---------- 5. 写 CHANGELOG.md（第 3 步生成的这一节） ---------- */

const HEADER = '# 更新日志\n\n版本号规则见 [AGENTS.md](AGENTS.md)「发版」。每一版由 `npm run release` 从上一版之后的提交生成。\n';
const file = join(root, 'CHANGELOG.md');
const old = existsSync(file) ? readFileSync(file, 'utf8') : HEADER;
const at = old.indexOf('\n## ');
writeFileSync(file, at < 0 ? `${old.trimEnd()}\n\n${notes}` : `${old.slice(0, at + 1)}${notes}\n${old.slice(at + 1)}`);

/* ---------- 6. 提交、打 tag ---------- */

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
// tag 说明：标题 + 这一版的 changelog（去掉「## vX.Y.Z · 日期」一行）。
// --cleanup=whitespace：默认会把 # 开头的行当注释删掉，「### 新功能」这类小标题就没了
git('tag', '-a', tag, '--cleanup=whitespace', '-m', `Board ${tag}`, '-m', notes.replace(/^## .*\n+/, '').trimEnd());
console.log(notes);
console.log(`已提交「chore: 发版 ${tag}」并打了 tag ${tag}（上一个 tag：${prev ?? '没有'}）。`);

/* ---------- 7. 推送 ---------- */

const cmds = [`git push origin HEAD:${BRANCH}`, `git push origin ${tag}`];
if (!push) {
  console.log(`\n确认无误后推送（推 tag 会部署线上服务端）：\n  ${cmds.join('\n  ')}`);
  process.exit(0);
}
const base = git('rev-parse', 'HEAD~1');
try {
  git('push', 'origin', `HEAD:${BRANCH}`);
} catch (e) {
  const why = String(e.stderr || e.message).trim();
  // 撤掉这次的发版提交和 tag（发版前工作区是干净的，HEAD~1 就是当时的 origin/main）
  git('tag', '-d', tag);
  git('reset', '-q', '--hard', base);
  let moved = false;
  try {
    git('fetch', 'origin');
    moved = git('rev-parse', REMOTE) !== base;
  } catch {
    // 连不上远端：按推送失败处理
  }
  if (!moved) fail(`推送 ${BRANCH} 失败，已撤掉这次的发版提交和 tag：\n${why}`);
  if (retried) fail(`${BRANCH} 又被别人先推了，已撤掉这次的发版提交和 tag。稍后重新执行：npm run release -- ${version} --push`);
  // 发版途中别人推了 main：重新拉到最新，再发一次
  console.log(`\n${BRANCH} 推送被拒：发版途中别人推了新提交。已撤掉这次的发版提交和 tag，拉到最新后重新发版。\n`);
  try {
    execFileSync(process.execPath, [fileURLToPath(import.meta.url), ...args, '--retried'], { cwd: root, stdio: 'inherit' });
    process.exit(0);
  } catch (err) {
    process.exit(err.status ?? 1);
  }
}
try {
  git('push', 'origin', tag);
} catch (e) {
  console.error(String(e.stderr || e.message).trim());
  fail(`${BRANCH} 已推送，但 tag ${tag} 没推上去（没推 tag 就不会部署）。重试：git push origin ${tag}`);
}
console.log(`已推送 ${BRANCH} 和 ${tag}，部署见 GitHub Actions → deploy。`);
