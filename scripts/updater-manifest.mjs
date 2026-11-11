/* ==========================================================================
   生成桌面端的更新清单 latest.json（Tauri updater 的静态清单格式）：

       {
         "version": "0.1.13",
         "notes": "<CHANGELOG.md 里这一版的段落>",
         "pub_date": "2026-11-11T06:00:00Z",
         "platforms": {
           "darwin-aarch64-app":   { "signature": "<.sig 的内容>", "url": ".../Board_0.1.13_universal.app.tar.gz" },
           "darwin-x86_64-app":    { "signature": "<.sig 的内容>", "url": ".../Board_0.1.13_universal.app.tar.gz" },
           "windows-x86_64-nsis":  { "signature": "<.sig 的内容>", "url": ".../Board_0.1.13_x64-setup.exe" }
         }
       }

   发版工作流（.github/workflows/deploy.yml 的 desktop 任务）在打好安装包后调用：tauri build（bundle.createUpdaterArtifacts）
   把每个平台的更新包和签名放在安装包旁边（target/<target>/release/bundle/macos/Board.app.tar.gz、bundle/nsis/<安装包>.exe）和 <文件>.sig；
   CI 把它们收进 updater/<平台键>/<文件>，这里读签名、写清单。release 时 latest.json 作为 GitHub Release 的附件上传，
   客户端从 https://github.com/<仓库>/releases/latest/download/latest.json 取（tauri.conf.json 的 plugins.updater.endpoints）。

   平台键就是 Tauri updater 用的 target：`<os>-<arch>[-<bundle 类型>]`，和安装包的类型一致
   （`windows-x86_64-nsis`、`darwin-aarch64-app`）。客户端在清单里没有自己平台的条目时，当作已是最新。

   用法（仓库根目录）：
     node scripts/updater-manifest.mjs --dir <updater 目录，下面按平台键分目录> --version <版本号> \
       --base <附件下载地址前缀> --out <latest.json 输出路径> [--pub-date <ISO 时间>] [--partial]
     （--partial：这台机器只带了部分平台，不要求齐全；发版时两个平台各写一份，release 里合并后要求齐全）

     node scripts/updater-manifest.mjs --dry-run      # 本机验证：拼一份清单、核对结构，改坏的要能被拦下
   ========================================================================== */
import { execFileSync } from 'node:child_process';
import { readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');

/** 参数：`--名字 值` 或 `--名字=值`；没有值的开关是 true */
export function parseArgs(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (!a.startsWith('--')) continue;
    const eq = a.indexOf('=');
    if (eq > 0) {
      out[a.slice(2, eq)] = a.slice(eq + 1);
    } else if (argv[i + 1] != null && !argv[i + 1].startsWith('--')) {
      out[a.slice(2)] = argv[++i];
    } else {
      out[a.slice(2)] = true;
    }
  }
  return out;
}

/** 清单里的版本号：去掉 v，只认 x.y.z（和 tag、tauri.conf.json、CHANGELOG 三处一致） */
export function manifestVersion(tag) {
  const v = String(tag ?? '').trim().replace(/^v/, '');
  if (!/^\d+\.\d+\.\d+$/.test(v)) throw new Error(`版本号「${tag}」不合法：要 x.y.z（tag 是 vx.y.z）`);
  return v;
}

/** 平台键是不是 Tauri 的 target（macOS 是 darwin、Windows 是 windows；后缀是安装包类型） */
export const TARGET_KEY = /^(darwin|windows)-[a-z0-9_]+(-[a-z0-9]+)?$/;

/** 更新包的扩展名：macOS 的 .app.tar.gz、Windows 的 NSIS .exe（msi 用不上，Tauri 的更新器不装 msi） */
const UPDATE_FILE = /\.(app\.tar\.gz|exe)$/;

/**
 * 读 updater 目录（严格按平台键分目录）：`updater/<平台键>/<文件>` 和同名的 `<文件>.sig`。
 * 返回 [{ target, file, url }]；没有签名、目录里有多个安装包、键不像平台都直接报错——
 * 宁可这次不发更新，也不要发出一份对不上的清单
 */
export function collect(dir, base) {
  if (!statSync(dir, { throwIfNoEntry: false })?.isDirectory()) throw new Error(`找不到 updater 目录 ${dir}`);
  const keys = readdirSync(dir)
    .filter(k => statSync(join(dir, k)).isDirectory())
    .sort();
  if (!keys.length) throw new Error(`${dir} 里没有平台目录`);
  const files = [];
  for (const key of keys) {
    if (!TARGET_KEY.test(key)) throw new Error(`平台目录「${key}」不像 Tauri 的 target（例如 windows-x86_64-nsis）`);
    const names = readdirSync(join(dir, key))
      .filter(n => !n.endsWith('.sig'))
      .filter(n => UPDATE_FILE.test(n))
      .sort();
    if (names.length !== 1) throw new Error(`updater/${key} 里应该有 1 个更新包，实际 ${names.length} 个：${names.join('、') || '没有'}`);
    const name = names[0];
    const sig = join(dir, key, `${name}.sig`);
    if (!statSync(sig, { throwIfNoEntry: false })?.isFile()) throw new Error(`updater/${key}/${name} 没有签名文件 ${name}.sig（构建时没配签名私钥？）`);
    try {
      readSignature(sig);
    } catch (e) {
      throw new Error(`updater/${key}/${name}.sig：${e.message}`);
    }
    files.push({ target: key, file: join(dir, key, name), url: `${base.replace(/\/$/, '')}/${encodeURIComponent(name)}` });
  }
  return files;
}

/**
 * 签名：tauri build 写的 `.sig` 是 minisign 签名文本（第一行 `untrusted comment:`）再做一次 base64，
 * 清单里原样放这串 base64（客户端先解 base64 再验 minisign）。去掉可能的 BOM 和首尾空白
 */
export function readSignature(path) {
  const text = readFileSync(path, 'utf8').replace(/^\uFEFF/, '').trim();
  if (!isSignature(text)) throw new Error(`${path} 不像 tauri 的签名（base64 编码的 minisign 签名）`);
  return text;
}

/** 是不是 base64 编码的 minisign 签名：只有 base64 字符，解开后第一行是 `untrusted comment:` */
export function isSignature(text) {
  const s = String(text ?? '');
  if (!s || !/^[A-Za-z0-9+/]+={0,2}$/.test(s)) return false;
  return Buffer.from(s, 'base64').toString('utf8').startsWith('untrusted comment:');
}

/** 拼清单：files 是 collect 的结果（或者测试里给的假条目）；签名读的是更新包旁边的 `<文件>.sig` */
export function buildManifest({ version, notes, pubDate, files, readSig = readSignature }) {
  const platforms = {};
  for (const f of [...files].sort((a, b) => a.target.localeCompare(b.target))) {
    if (platforms[f.target]) throw new Error(`清单里 ${f.target} 出现了两次`);
    platforms[f.target] = { signature: readSig(`${f.file}.sig`), url: f.url };
  }
  return { version, notes, pub_date: pubDate, platforms };
}

/** CHANGELOG.md 里这一版的一节（和 GitHub Release 的正文同一段）；没有就报错 */
export function readNotes(version, changelog = join(root, 'CHANGELOG.md')) {
  const lines = readFileSync(changelog, 'utf8').split(/\r?\n/);
  const start = lines.findIndex(l => /^## v\d+\.\d+\.\d+(\s|$)/.test(l) && l.startsWith(`## v${version}`));
  if (start < 0) throw new Error(`CHANGELOG.md 里没有 v${version} 一节`);
  let end = lines.findIndex((l, i) => i > start && l.startsWith('## '));
  if (end < 0) end = lines.length;
  return lines.slice(start + 1, end).join('\n').trim();
}

/** 发版前客户端能装的平台：macOS 两个架构 + Windows x64，缺了不发 Release（见 deploy.yml 的检查） */
export const REQUIRED_TARGETS = ['darwin-aarch64-app', 'darwin-x86_64-app', 'windows-x86_64-nsis'];

/** 核对清单结构；给了 required 时这些平台都必须有。返回 { problems, summary }（problems 为空才算合格） */
export function validateManifest(m, { expectVersion, required = REQUIRED_TARGETS } = {}) {
  const problems = [];
  if (!/^\d+\.\d+\.\d+$/.test(m?.version ?? '')) problems.push(`version「${m?.version}」不是 x.y.z`);
  if (expectVersion && m?.version !== expectVersion) problems.push(`version ${m?.version} 和 tag（${expectVersion}）不一致`);
  if (typeof m?.notes !== 'string') problems.push('notes 不是字符串');
  if (!m?.pub_date || Number.isNaN(Date.parse(m.pub_date))) problems.push(`pub_date「${m?.pub_date}」不是 ISO 时间`);
  const platforms = m?.platforms && typeof m.platforms === 'object' ? m.platforms : {};
  const keys = Object.keys(platforms);
  if (!keys.length) problems.push('platforms 是空的');
  for (const [k, p] of Object.entries(platforms)) {
    if (!TARGET_KEY.test(k)) problems.push(`platforms 的键「${k}」不像 Tauri 的 target`);
    if (!isSignature(p?.signature)) problems.push(`${k} 没有签名，或者签名不是 .sig 的内容（base64 编码的 minisign 签名）`);
    let url;
    try {
      url = new URL(p?.url ?? '');
    } catch {
      problems.push(`${k} 的 url「${p?.url}」不合法`);
      continue;
    }
    if (url.protocol !== 'https:') problems.push(`${k} 的 url 不是 https`);
    if (!UPDATE_FILE.test(decodeURIComponent(url.pathname))) problems.push(`${k} 的 url 不指向更新包（${url.pathname}）`);
  }
  const missing = required.filter(k => !platforms[k]);
  if (keys.length && missing.length) problems.push(`缺少平台的更新包：${missing.join('、')}`);
  const summary = keys.map(k => `  ${k} → ${platforms[k].url}`).join('\n');
  return { problems, summary };
}

/* ---------------- 命令行 ---------------- */

function main() {
  const a = parseArgs(process.argv.slice(2));
  if (a['dry-run']) return dryRun();
  for (const k of ['dir', 'version', 'base', 'out']) {
    if (!a[k] || a[k] === true) throw new Error(`缺参数 --${k}（用法见脚本开头）`);
  }
  const version = manifestVersion(a.version);
  const notes = readNotes(version);
  const files = collect(resolve(a.dir), a.base);
  const pubDate = a['pub-date'] || new Date().toISOString().replace(/\.\d{3}Z$/, 'Z');
  const m = buildManifest({ version, notes, pubDate, files });
  // --partial：只管这一个平台的清单（发版时 macOS、Windows 各写一份，release 里合并成一份再要求齐全）
  const { problems, summary } = validateManifest(m, { expectVersion: version, required: a.partial ? [] : REQUIRED_TARGETS });
  if (problems.length) throw new Error(`清单不对，不写：\n${problems.map(p => `  - ${p}`).join('\n')}`);
  writeFileSync(a.out, JSON.stringify(m, null, 2) + '\n');
  console.log(`✓ ${a.out}（v${version}，CHANGELOG ${notes.length} 字）\n${summary}`);
}

/**
 * 本机验证（CI 的 test 任务也跑）：用假签名拼一份清单，核对结构；再故意写坏几处，确认能被拦下。
 * 不发版时 CHANGELOG 里还没有「下一个版本」的一节，所以说明用临时文字
 */
function dryRun() {
  const version = manifestVersion(process.env.BOARD_VERSION ?? execFileSync('node', ['-p', "require('./src-tauri/tauri.conf.json').version"], { cwd: root, encoding: 'utf8' }));
  // 和 tauri build 写的 .sig 一样：minisign 签名文本再做一次 base64
  const sig = Buffer.from('untrusted comment: signature from tauri secret key\nRWfake1\ntrusted comment: fake\nfake2\n').toString('base64');
  const base = `https://github.com/buglan/board/releases/download/v${version}`;
  const files = REQUIRED_TARGETS.map(t => ({ target: t, file: 'fake', url: `${base}/${t.endsWith('-app') ? `Board_${version}_universal.app.tar.gz` : `Board_${version}_x64-setup.exe`}` }));
  // 顺手核对 CHANGELOG 里有没有这一版（BOARD_VERSION=<下一个版本号> 时核对下一个版本；没发版时不检查）
  let notes = `## v${version}\n- 示例`;
  let notesNote = 'CHANGELOG 还没发版，用示例说明';
  try {
    notes = readNotes(version);
    notesNote = `CHANGELOG 里 v${version} 一节 ${notes.length} 字`;
  } catch {
    if (process.env.BOARD_VERSION) throw new Error(`CHANGELOG.md 里没有 v${version} 一节（BOARD_VERSION 指定了）`);
  }
  const m = buildManifest({ version, notes, pubDate: '2026-11-11T06:00:00Z', files, readSig: () => sig });
  const { problems, summary } = validateManifest(m, { expectVersion: version });

  // 写坏的清单要被拦下：签名空、地址不是 https、缺平台、版本对不上
  const bad = [
    () => ({ ...m, version: 'nightly' }),
    () => ({ ...m, platforms: { ...m.platforms, 'windows-x86_64-nsis': { signature: '', url: m.platforms['windows-x86_64-nsis'].url } } }),
    () => ({ ...m, platforms: { ...m.platforms, 'darwin-aarch64-app': { signature: sig, url: 'file:///etc/passwd' } } }),
    () => ({ ...m, platforms: { 'windows-x86_64-nsis': m.platforms['windows-x86_64-nsis'] } }),
    // 没解 base64 的 minisign 文本（客户端不认）
    () => ({ ...m, platforms: { ...m.platforms, 'darwin-x86_64-app': { signature: Buffer.from(sig, 'base64').toString('utf8'), url: m.platforms['darwin-x86_64-app'].url } } }),
  ];
  for (const [i, make] of bad.entries()) {
    if (!validateManifest(make(), { expectVersion: version }).problems.length) problems.push(`写坏的清单（第 ${i + 1} 个）没有被拦下`);
  }
  if (JSON.parse(JSON.stringify(m)).platforms['darwin-x86_64-app'].signature !== sig) problems.push('签名经过 JSON 序列化后变了');
  console.log(`✓ latest.json 结构（v${version}；${notesNote}）\n${summary}\n✓ 写坏的清单都被拦下`);
  if (problems.length) throw new Error(`dry-run 失败：\n${problems.map(p => `  - ${p}`).join('\n')}`);
  console.log('✓ dry-run 通过');
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (e) {
    console.error(`::error::${e.message}`);
    process.exit(1);
  }
}