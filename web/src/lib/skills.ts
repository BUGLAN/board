/* ==========================================================================
   skills 共用库（PRD §10.12「共用配置」）：界面上用到的纯函数
   - 从机器导入：一台机器上不是同步装的 skill 和库里的比一比（importRows）
   ========================================================================== */
import type { LocalSkill, SkillInfo } from './types';

/** 从机器导入时一个本机 skill 的情况：new 库里没有 / differs 库里有同名、内容不一样（导入会换成这份）/ same 和库里一样（不用导入）/ bad 不能导入（写了原因） */
export type ImportKind = 'new' | 'differs' | 'same' | 'bad';
export type ImportRow = { skill: LocalSkill; kind: ImportKind; note: string };

export function importRows(local: LocalSkill[], lib: SkillInfo[]): ImportRow[] {
  return local.map(skill => {
    if (skill.why) return { skill, kind: 'bad', note: skill.why };
    const have = lib.find(s => s.name === skill.name);
    if (!have) return { skill, kind: 'new', note: '库里没有' };
    if (have.builtin) return { skill, kind: 'bad', note: '随服务端版本，不能导入' };
    if (have.version === skill.version) return { skill, kind: 'same', note: '和库里的一样' };
    return { skill, kind: 'differs', note: `库里有同名的（${have.version.slice(0, 7)}）：导入会换成这份` };
  });
}
