import { describe, expect, it } from 'vitest';
import { importRows, skillRows } from './skills';
import type { HostInfo, SkillInfo } from './types';

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
  });
});

describe('从机器导入', () => {
  it('库里没有、内容不一样、和库里一样、不能导入', () => {
    const lib = [
      { name: 'board-skills', version: 'bbbbbbbbbbbb', size: 1, files: 1, updatedAt: 0, by: '', source: '', builtin: true },
      { name: 'design', version: 'v2v2v2v2v2v2', size: 1, files: 1, updatedAt: 0, by: '', source: '' },
      { name: 'notes', version: 'n1n1n1n1n1n1', size: 1, files: 1, updatedAt: 0, by: '', source: '' },
    ];
    const local = (name: string, version: string, why?: string) => ({ name, version, files: 1, size: 1, agents: ['claude'], dirs: ['~/.claude/skills'], why });
    const rows = importRows([local('fresh', 'f1f1f1f1f1f1'), local('design', 'v1v1v1v1v1v1'), local('notes', 'n1n1n1n1n1n1'), local('huge', '', '超过 5 MB'), local('board-skills', 'x')], lib);
    expect(rows.map(r => r.kind)).toEqual(['new', 'differs', 'same', 'bad', 'bad']);
    expect(rows[1].note).toContain('v2v2v2v');
  });
});
