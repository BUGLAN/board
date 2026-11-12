import { describe, expect, it } from 'vitest';
import { packFiles, readZip, skillRows, skillSummary, SKILL_MAX } from './skills';
import type { HostInfo, SkillInfo } from './types';

const enc = new TextEncoder();
const f = (path: string, s: string, exec = false) => ({ path, data: enc.encode(s), exec });

/** 测试用的 zip：stored 或 deflate，带 Unix 权限 */
async function makeZip(entries: { name: string; data: string; mode?: number }[], deflate: boolean): Promise<ArrayBuffer> {
  const parts: Uint8Array[] = [];
  const central: Uint8Array[] = [];
  let off = 0;
  for (const e of entries) {
    const raw = enc.encode(e.data);
    const body = deflate ? new Uint8Array(await new Response(new Response(raw).body!.pipeThrough(new CompressionStream('deflate-raw'))).arrayBuffer()) : raw;
    const name = enc.encode(e.name);
    const local = new DataView(new ArrayBuffer(30));
    local.setUint32(0, 0x04034b50, true);
    local.setUint16(8, deflate ? 8 : 0, true);
    local.setUint32(18, body.length, true);
    local.setUint32(22, raw.length, true);
    local.setUint16(26, name.length, true);
    const cen = new DataView(new ArrayBuffer(46));
    cen.setUint32(0, 0x02014b50, true);
    cen.setUint16(10, deflate ? 8 : 0, true);
    cen.setUint32(20, body.length, true);
    cen.setUint32(24, raw.length, true);
    cen.setUint16(28, name.length, true);
    cen.setUint32(38, ((e.mode ?? 0o644) << 16) >>> 0, true);
    cen.setUint32(42, off, true);
    parts.push(new Uint8Array(local.buffer), name, body);
    central.push(new Uint8Array(cen.buffer), name);
    off += 30 + name.length + body.length;
  }
  const csize = central.reduce((n, c) => n + c.length, 0);
  const end = new DataView(new ArrayBuffer(22));
  end.setUint32(0, 0x06054b50, true);
  end.setUint16(8, entries.length, true);
  end.setUint16(10, entries.length, true);
  end.setUint32(12, csize, true);
  end.setUint32(16, off, true);
  const all = [...parts, ...central, new Uint8Array(end.buffer)];
  const out = new Uint8Array(all.reduce((n, p) => n + p.length, 0));
  let i = 0;
  for (const p of all) {
    out.set(p, i);
    i += p.length;
  }
  return out.buffer;
}

describe('上传 skill：整理文件', () => {
  it('选目录：去掉顶层目录当名称，跳过隐藏文件，内容转成 base64', () => {
    const r = packFiles([f('My-Notes/SKILL.md', '# n'), f('My-Notes/ref/a.md', 'a'), f('My-Notes/.DS_Store', 'x'), f('My-Notes/.git/HEAD', 'x')], 'x');
    expect('error' in r).toBe(false);
    if ('error' in r) return;
    expect(r.name).toBe('my-notes');
    expect(r.files.map(x => x.path)).toEqual(['SKILL.md', 'ref/a.md']);
    expect(atob(r.files[0].data)).toBe('# n');
  });
  it('zip 里直接是文件：用 zip 的文件名当名称', () => {
    const r = packFiles([f('SKILL.md', '# n')], 'notes.zip');
    expect('error' in r ? r.error : r.name).toBe('notes');
  });
  it('没有 SKILL.md、超过 5 MB、名称不对、board-skills：拒绝并说明', () => {
    expect(packFiles([f('notes/README.md', 'x')], 'x')).toMatchObject({ error: expect.stringContaining('SKILL.md') });
    const big = { path: 'n/blob', data: new Uint8Array(SKILL_MAX + 1) };
    expect(packFiles([f('n/SKILL.md', 'x'), big], 'x')).toMatchObject({ error: expect.stringContaining('超过 5 MB') });
    expect(packFiles([f('My Notes/SKILL.md', 'x')], 'x')).toMatchObject({ error: expect.stringContaining('名称') });
    expect(packFiles([f('board-skills/SKILL.md', 'x')], 'x')).toMatchObject({ error: expect.stringContaining('随服务端版本') });
  });
});

describe('读 zip', () => {
  it('不压缩、deflate 都读得出，记下可执行位', async () => {
    for (const deflate of [false, true]) {
      const zip = await makeZip(
        [
          { name: 'notes/SKILL.md', data: '# 笔记' },
          { name: 'notes/scripts/run', data: '#!/bin/sh\necho hi', mode: 0o755 },
        ],
        deflate,
      );
      const files = await readZip(zip);
      expect(files.map(x => [x.path, new TextDecoder().decode(x.data), x.exec])).toEqual([
        ['notes/SKILL.md', '# 笔记', false],
        ['notes/scripts/run', '#!/bin/sh\necho hi', true],
      ]);
    }
  });
  it('不是 zip 时报错', async () => {
    await expect(readZip(new ArrayBuffer(40))).rejects.toThrow('不是 zip');
  });
});

describe('同步状态', () => {
  const skill: SkillInfo = { name: 'notes', version: 'v2', size: 1, files: 1, updatedAt: 1, by: 'a', source: 'mbp' };
  const host = (name: string, x: Partial<HostInfo>): HostInfo =>
    ({ id: 1, name, online: true, agents: [{ id: 'claude', name: 'Claude Code', found: true, version: '1' }], ...x }) as HostInfo;
  it('旧版不同步、还没报的是正在同步、已同步但版本旧的算落后', () => {
    const rows = skillRows(
      [
        host('old', { skills: null }),
        host('new', { skills: [] }),
        host('ok', { skills: [{ skill: 'notes', agent: 'claude', env: 'login', dir: '~/.claude/skills', state: 'synced', version: 'v2' }] }),
        host('lag', { skills: [{ skill: 'notes', agent: 'claude', env: 'login', dir: '~/.claude/skills', state: 'synced', version: 'v1' }] }),
        host('mine', { skills: [{ skill: 'notes', agent: 'claude', env: 'login', dir: '~/.claude/skills', state: 'local', why: '本机已有同名的目录' }] }),
      ],
      skill,
    );
    expect(rows.map(r => (r.old ? 'old' : r.cells.map(c => c.state).join()))).toEqual(['old', 'pending', 'synced', 'behind', 'local']);
    expect(skillSummary(rows)).toEqual({ synced: 1, bad: 2, old: 1 });
  });
});
