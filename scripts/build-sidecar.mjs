/* ==========================================================================
   把 board 命令行编译好，放进 src-tauri/binaries，供 Tauri 的 bundle.externalBin
   随桌面端安装包一起打包：
   - Windows 安装目录里多一个 board.exe（安装器把它写进 PATH）
   - macOS 的 Board.app/Contents/MacOS 里多一个 board
   Tauri 只认带目标三元组的文件名（board-<triple>.exe），这里负责编译和改名。

   npm run desktop:build 之前自动跑；桌面端开发（desktop:dev）跑 --debug。
   ========================================================================== */
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const debug = process.argv.includes('--debug');
const exe = process.platform === 'win32' ? '.exe' : '';

/** rustc 自带的目标三元组（x86_64-pc-windows-msvc、aarch64-apple-darwin 等） */
function hostTriple() {
  const out = execFileSync('rustc', ['-Vv'], { encoding: 'utf8' });
  const m = out.match(/^host:\s*(\S+)$/m);
  if (!m) throw new Error('读不到 rustc 的 host triple，确认 Rust 已经装好');
  return m[1];
}

const host = hostTriple();
// Tauri 用 --target 交叉编译时会通过这个环境变量告诉构建脚本目标平台
const triple = process.env.TAURI_ENV_TARGET_TRIPLE || host;

const args = ['build', '-p', 'board-cli'];
if (!debug) args.push('--release');
if (triple !== host) args.push('--target', triple);
execFileSync('cargo', args, { cwd: root, stdio: 'inherit' });

// 本机编译时产物在 target/<profile>，交叉编译时在 target/<triple>/<profile>
const profile = debug ? 'debug' : 'release';
const out = join(root, 'target', triple === host ? '' : triple, profile, 'board' + exe);
const destDir = join(root, 'src-tauri', 'binaries');
const dest = join(destDir, `board-${triple}${exe}`);
mkdirSync(destDir, { recursive: true });
copyFileSync(out, dest);
console.log(`✓ board 命令行已就绪：${dest}`);
