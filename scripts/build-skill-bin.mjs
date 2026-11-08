/* ==========================================================================
   把 board 命令行编译成各平台的二进制，放进 skill 目录随 skill 分发：
     .agents/skills/board-skills/bin/<平台>/board[.exe]
   skill 里的 scripts/board（Windows：scripts\board.cmd）按平台选用，agent 的电脑不用另装 board。
   bin/ 不进 git；发版时 CI 在 macOS 上跑 `--zip`，把整个 skill 目录打成 board-skills.zip 附在 GitHub Release 上。

   用法：
     node scripts/build-skill-bin.mjs            本机能编的平台都编
     node scripts/build-skill-bin.mjs --host     只编本机平台（开发时用）
     node scripts/build-skill-bin.mjs --zip      编完再打包 target/board-skills.zip
   交叉编译：macOS 的两个平台要在 macOS 上编（rustup target add aarch64-apple-darwin x86_64-apple-darwin）；
   Linux（musl 静态链接）和 Windows 用 zig 编（brew install zig、cargo install cargo-zigbuild，
   rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl x86_64-pc-windows-gnu）。
   ========================================================================== */
import { execFileSync } from 'node:child_process';
import { chmodSync, copyFileSync, mkdirSync, rmSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const skill = join(root, '.agents', 'skills', 'board-skills');
const hostOnly = process.argv.includes('--host');
const zip = process.argv.includes('--zip');

/** 平台目录名 → 目标三元组；zig：用 cargo zigbuild 交叉编译 */
const PLATFORMS = [
  { dir: 'darwin-arm64', triple: 'aarch64-apple-darwin', os: 'darwin' },
  { dir: 'darwin-x64', triple: 'x86_64-apple-darwin', os: 'darwin' },
  { dir: 'linux-x64', triple: 'x86_64-unknown-linux-musl', os: 'linux', zig: true },
  { dir: 'linux-arm64', triple: 'aarch64-unknown-linux-musl', os: 'linux', zig: true },
  { dir: 'windows-x64', triple: 'x86_64-pc-windows-gnu', os: 'win32', zig: true, exe: '.exe' },
];

const has = (cmd, args = ['--version']) => {
  try {
    execFileSync(cmd, args, { stdio: 'ignore' });
    return true;
  } catch {
    return false;
  }
};

const hostDir = `${{ darwin: 'darwin', linux: 'linux', win32: 'windows' }[process.platform]}-${process.arch === 'arm64' ? 'arm64' : 'x64'}`;
const zigOk = has('cargo', ['zigbuild', '--help']) && has('zig', ['version']);

const built = [];
const skipped = [];
for (const p of PLATFORMS) {
  if (hostOnly && p.dir !== hostDir) continue;
  // macOS 的二进制只能在 macOS 上编；Linux、Windows 一律用 zig 编（静态链接，不依赖本机的 C 工具链）
  if (p.os === 'darwin' && process.platform !== 'darwin') {
    skipped.push(`${p.dir}（要在 macOS 上编）`);
    continue;
  }
  const useZig = !!p.zig;
  if (useZig && !zigOk) {
    skipped.push(`${p.dir}（没装 zig / cargo-zigbuild）`);
    continue;
  }
  const args = [useZig ? 'zigbuild' : 'build', '--release', '--locked', '-p', 'board-cli', '--target', p.triple];
  console.log(`→ ${p.dir}：cargo ${args.join(' ')}`);
  execFileSync('cargo', args, { cwd: root, stdio: 'inherit' });
  const exe = p.exe ?? '';
  const destDir = join(skill, 'bin', p.dir);
  mkdirSync(destDir, { recursive: true });
  const dest = join(destDir, `board${exe}`);
  copyFileSync(join(root, 'target', p.triple, 'release', `board${exe}`), dest);
  chmodSync(dest, 0o755);
  built.push(p.dir);
}

console.log(`✓ 已放进 ${join('.agents', 'skills', 'board-skills', 'bin')}：${built.join('、') || '（没有）'}`);
if (skipped.length) console.log(`  跳过：${skipped.join('、')}`);
if (!built.length) process.exit(1);

if (zip) {
  if (skipped.length) {
    console.error('✗ 打包要求所有平台都编好，先装好上面跳过的工具链');
    process.exit(1);
  }
  const out = join(root, 'target', 'board-skills.zip');
  rmSync(out, { force: true });
  // 在 skills 目录里打包，解压后是 board-skills/ 目录；zip 会保留可执行权限
  execFileSync('zip', ['-qr', out, 'board-skills', '-x', '*.DS_Store'], { cwd: dirname(skill), stdio: 'inherit' });
  console.log(`✓ ${out}`);
}
