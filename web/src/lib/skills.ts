/* ==========================================================================
   skills 共用库（PRD §10.12「共用配置」）：界面上用到的纯函数
   - 每个 skill 在各台机器、各个 agent 上的同步状态（skillRows）
   - 从机器导入：一台机器上不是同步装的 skill 和库里的比一比（importRows）
   ========================================================================== */
import type { HostInfo, LocalSkill, SkillInfo, SkillState } from './types';

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

export const STATE_WORDS: Record<SyncCell['state'], string> = {
  synced: '已同步',
  behind: '版本落后',
  local: '本机有同名，没有覆盖',
  failed: '失败',
  pending: '正在同步',
};

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
