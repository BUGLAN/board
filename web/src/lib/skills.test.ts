import { describe, expect, it } from 'vitest';
import { importRows } from './skills';

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
