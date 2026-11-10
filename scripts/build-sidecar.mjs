/* ==========================================================================
   把 board 命令行编译好，放进 src-tauri/binaries，供 Tauri 的 bundle.externalBin
   随桌面端安装包一起打包：
   - Windows 安装目录里多一个 board.exe（安装器把它写进 PATH）
   - macOS 的 Board.app/Contents/MacOS 里多一个 board
   Tauri 只认带目标三元组的文件名（board-<triple>.exe），这里负责编译和改名。

   npm run desktop:build 之前自动跑；桌面端开发（desktop:dev）跑 --debug。
   macOS 通用包（tauri build --target universal-apple-darwin，发版工作流用）：分别编 arm64 和 x64，
   再用 lipo 合成 board-universal-apple-darwin；两个单架构的文件也留着（Tauri 编各架构时的构建脚本要找它们）。
   ========================================================================== */
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const debug = process.argv.includes('--debug');

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

/**
 * Windows 上先结束上次开发遗留的桌面端和 board host。
 *
 * Tauri dev 会把 sidecar 复制到 target/debug/board.exe，cargo build 也要写这个文件；
 * 桌面端「让这台电脑接任务」打开的 board host 正是从这里启动的，它不退出，
 * cargo 就会报 `failed to remove file ... board.exe`（os error 5，拒绝访问）。
 * 只按本仓库 target/debug 下的完整路径匹配，不会碰安装版或别的 checkout。
 */
function stopStaleDevProcesses() {
  if (process.platform !== 'win32' || !debug) return;
  const targets = [join(root, 'target', 'debug', 'board-desktop.exe'), join(root, 'target', 'debug', 'board.exe')];
  const script = `
$targets = @($env:BOARD_DEV_TARGETS.Split(';'))
$procs = @(Get-Process -Name board-desktop,board -ErrorAction SilentlyContinue | Where-Object { $_.Path -and ($targets -contains $_.Path) })
if ($procs.Count -eq 0) { return }
$procs | ForEach-Object { Write-Output ("{0}|{1}" -f $_.Id, $_.ProcessName) }
$procs | Stop-Process -Force -ErrorAction SilentlyContinue
$deadline = (Get-Date).AddSeconds(5)
while ((Get-Date) -lt $deadline) {
  $left = @(Get-Process -Name board-desktop,board -ErrorAction SilentlyContinue | Where-Object { $_.Path -and ($targets -contains $_.Path) })
  if ($left.Count -eq 0) { return }
  Start-Sleep -Milliseconds 100
}
throw "仍有进程占用 target\\debug\\board.exe：" + (($left | ForEach-Object { "$($_.ProcessName) (PID $($_.Id))" }) -join '、')
`;
  const out = execFileSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script], {
    encoding: 'utf8',
    env: { ...process.env, BOARD_DEV_TARGETS: targets.join(';') },
    windowsHide: true,
  }).trim();
  if (out) {
    const names = out
      .split(/\r?\n/)
      .filter(Boolean)
      .map((line) => {
        const [pid, name] = line.split('|');
        return `${name}.exe (PID ${pid})`;
      });
    console.log(`… 已结束上次开发遗留的进程：${names.join('、')}`);
  }
}

stopStaleDevProcesses();

const profile = debug ? 'debug' : 'release';
const destDir = join(root, 'src-tauri', 'binaries');
mkdirSync(destDir, { recursive: true });

/** 编一个平台的 board，放成 board-<triple>，返回放到的路径 */
function build(t) {
  const args = ['build', '-p', 'board-cli'];
  if (!debug) args.push('--release');
  if (t !== host) args.push('--target', t);
  execFileSync('cargo', args, { cwd: root, stdio: 'inherit' });
  // 本机编译时产物在 target/<profile>，交叉编译时在 target/<triple>/<profile>
  const ext = t.includes('windows') ? '.exe' : '';
  const out = join(root, 'target', t === host ? '' : t, profile, 'board' + ext);
  const dest = join(destDir, `board-${t}${ext}`);
  copyFileSync(out, dest);
  return dest;
}

let dest;
if (triple === 'universal-apple-darwin') {
  const parts = ['aarch64-apple-darwin', 'x86_64-apple-darwin'].map(build);
  dest = join(destDir, `board-${triple}`);
  execFileSync('lipo', ['-create', '-output', dest, ...parts], { stdio: 'inherit' });
} else {
  dest = build(triple);
}
console.log(`✓ board 命令行已就绪：${dest}`);
