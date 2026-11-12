/* ==========================================================================
   skills 共用库（PRD §10.12「共用配置」）：界面上用到的纯函数
   - 每个 skill 在各台机器、各个 agent 上的同步状态（skillRows）
   - 上传：选一个目录（webkitdirectory）或一个 zip，整理成 [{path, exec, data(base64)}]（packFiles、readZip）
   ========================================================================== */
import type { HostInfo, SkillInfo, SkillState } from './types';

/** 和服务端一致：一个 skill 合计最大 5 MB */
export const SKILL_MAX = 5 * 1024 * 1024;

export type SyncCell = { agent: string; state: SkillState['state'] | 'pending'; why?: string | null; version?: string | null; dir?: string };
export type SyncRow = { host: HostInfo; old: boolean; cells: SyncCell[] };

/** 一个 skill 在每台机器上的状态：旧版 board host 不同步（old）；新版还没报这个 skill 的是「正在同步」 */
export function skillRows(hosts: HostInfo[], skill: SkillInfo): SyncRow[] {
  return hosts.map(h => {
    if (h.skills == null) return { host: h, old: true, cells: [] };
    const mine = h.skills.filter(s => s.skill === skill.name);
    const agents = h.agents.filter(a => a.found).map(a => a.id);
    const cells: SyncCell[] = mine.map(s => ({
      agent: s.agent,
      // 报的是已同步、但版本不是库里现在的：还没跟上
      state: s.state === 'synced' && s.version && s.version !== skill.version ? 'behind' : s.state,
      why: s.why,
      version: s.version,
      dir: s.dir,
    }));
    // 找到的 agent 还没有这个 skill 的状态（刚上传，或这个 agent 没有用户级 skill 目录）
    if (!cells.length && agents.length && h.online) cells.push({ agent: '', state: 'pending' });
    return { host: h, old: false, cells };
  });
}

/** 一个 skill 的汇总：几台机器已同步、几处有问题 */
export function skillSummary(rows: SyncRow[]) {
  let synced = 0;
  let bad = 0;
  let old = 0;
  for (const r of rows) {
    if (r.old) old++;
    else if (r.cells.length && r.cells.every(c => c.state === 'synced')) synced++;
    else if (r.cells.some(c => c.state === 'failed' || c.state === 'behind' || c.state === 'local')) bad++;
  }
  return { synced, bad, old };
}

export const STATE_WORDS: Record<SyncCell['state'], string> = {
  synced: '已同步',
  behind: '版本落后',
  local: '本机有同名，没有覆盖',
  failed: '失败',
  pending: '正在同步',
};

/** 名称：小写字母、数字、. _ -，以字母或数字开头（和服务端一致） */
export function skillNameError(name: string): string | null {
  if (!/^[a-z0-9][a-z0-9._-]{0,63}$/.test(name)) return `skill 名称「${name}」不对：只能用小写字母、数字、.、_、-，以字母或数字开头`;
  if (name === 'board-skills') return 'board-skills 随服务端版本，不能上传';
  return null;
}

export function toBase64(b: Uint8Array): string {
  let s = '';
  for (let i = 0; i < b.length; i += 0x8000) s += String.fromCharCode(...b.subarray(i, i + 0x8000));
  return btoa(s);
}

export type RawFile = { path: string; data: Uint8Array; exec?: boolean };

/**
 * 整理上传的文件：路径统一用 /；只有一个顶层目录时去掉它（选目录、zip 里套了一层）并把它当作名称；
 * 跳过以 . 开头的（.git、.DS_Store、__MACOSX）；要有 SKILL.md；合计不超过 5 MB
 */
export function packFiles(raw: RawFile[], fallbackName: string): { name: string; files: { path: string; exec: boolean; data: string }[] } | { error: string } {
  let list = raw.map(f => ({ ...f, path: f.path.replace(/\\/g, '/').replace(/^\/+/, '') })).filter(f => f.path && !f.path.endsWith('/'));
  list = list.filter(f => !f.path.split('/').some(s => s.startsWith('.') || s === '__MACOSX'));
  let name = fallbackName;
  const tops = new Set(list.map(f => f.path.split('/')[0]));
  if (tops.size === 1 && list.every(f => f.path.includes('/'))) {
    name = [...tops][0];
    list = list.map(f => ({ ...f, path: f.path.slice(name.length + 1) }));
  }
  name = name.toLowerCase().replace(/\.zip$/, '');
  const err = skillNameError(name);
  if (err) return { error: err };
  if (!list.some(f => f.path === 'SKILL.md')) return { error: `「${name}」里没有 SKILL.md：上传的是 skill 目录本身（里面直接有 SKILL.md）` };
  const size = list.reduce((n, f) => n + f.data.length, 0);
  if (size > SKILL_MAX) return { error: `「${name}」有 ${(size / 1024 / 1024).toFixed(1)} MB，超过 5 MB：大文件不要放进 skill 目录` };
  return { name, files: list.map(f => ({ path: f.path, exec: !!f.exec, data: toBase64(f.data) })) };
}

/** 读 zip（只认不压缩和 deflate 两种，常见的 zip 都是这两种）；解不开时抛出原因 */
export async function readZip(buf: ArrayBuffer): Promise<RawFile[]> {
  const v = new DataView(buf);
  const u8 = new Uint8Array(buf);
  // 中央目录结尾：从末尾往前找签名 0x06054b50
  let end = -1;
  for (let i = buf.byteLength - 22; i >= Math.max(0, buf.byteLength - 65557); i--) {
    if (v.getUint32(i, true) === 0x06054b50) {
      end = i;
      break;
    }
  }
  if (end < 0) throw new Error('不是 zip 文件');
  const count = v.getUint16(end + 10, true);
  let p = v.getUint32(end + 16, true);
  const out: RawFile[] = [];
  const dec = new TextDecoder();
  for (let n = 0; n < count; n++) {
    if (v.getUint32(p, true) !== 0x02014b50) throw new Error('zip 的目录坏了');
    const method = v.getUint16(p + 10, true);
    const csize = v.getUint32(p + 20, true);
    const nlen = v.getUint16(p + 28, true);
    const xlen = v.getUint16(p + 30, true);
    const clen = v.getUint16(p + 32, true);
    const attr = v.getUint32(p + 38, true);
    const local = v.getUint32(p + 42, true);
    const name = dec.decode(u8.subarray(p + 46, p + 46 + nlen));
    p += 46 + nlen + xlen + clen;
    if (name.endsWith('/')) continue;
    const lstart = local + 30 + v.getUint16(local + 26, true) + v.getUint16(local + 28, true);
    const body = u8.subarray(lstart, lstart + csize);
    let data: Uint8Array;
    if (method === 0) data = body.slice();
    else if (method === 8) data = new Uint8Array(await new Response(new Response(body).body!.pipeThrough(new DecompressionStream('deflate-raw'))).arrayBuffer());
    else throw new Error(`zip 里的 ${name} 用了不支持的压缩方式（${method}）：用普通的 zip 重新打包`);
    // 高 16 位是 Unix 权限：有可执行位的记下来
    out.push({ path: name, data, exec: ((attr >>> 16) & 0o111) !== 0 });
  }
  return out;
}
