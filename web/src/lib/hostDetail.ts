/* ==========================================================================
   机器详情（机器详情与配置同步重构）：界面上用到的纯函数
   - 一个 agent 正不正常（agentHealth）：没找到、看不到 board-skills、配置没写进去、自检没过；页签上的数字按它数
   - 一个 agent 的配置写进去没有（configLine）：已写入 / 正在写入 / 不改 / 服务器上没给它配 / 机器离线 / 写入失败
   - skills、MCP 各一行（skillsLine、mcpLine）：用服务器的写了几个、写进了哪里；不改时写 Board 不碰
   - 工作空间只列例外（workspaceExceptions）：--dir 单独指定的、克隆失败的、不是 git 仓库的；还没建的项目并进一行（laterProjects）
   - 今天交审几张、异常退出几次（todayCounts）
   ========================================================================== */
import { dirFresh, since, skillMiss } from './hosts';
import { runPick, runWords, type RunPick } from './providers';
import type { Dispatch, HostAgent, HostInfo, ProviderView } from './types';

/* ------------------------------ agent ------------------------------ */

export interface AgentHealth {
  ok: boolean;
  /** 右上角的状态：没过时写什么 */
  chip?: string;
  /** 一行原因（配置没写进去时原因写在写入状态那一行，这里为空） */
  why?: string;
  /** 原因旁边的按钮 */
  act?: 'sync' | 'selftest';
  actLabel?: string;
  /** 自检的时间 */
  at?: number;
}

/** 一个 agent：找到了、看得到 board-skills、配置写进去了、自检能回话 */
export function agentHealth(h: HostInfo, a: HostAgent): AgentHealth {
  if (!a.found) return { ok: false, chip: '这台机器上没找到' };
  const t = h.selftest?.items.find(i => i.kind === 'agent' && i.key === a.id);
  if (skillMiss(h, a)) return { ok: false, chip: '缺 board-skills', why: '看不到 board-skills：派给它的卡会做不成', act: 'sync', actLabel: '装上' };
  const st = h.sync?.last?.agents.find(s => s.agent === a.id);
  if (st?.state === 'failed' && (a.run_mode ?? 'off') !== 'off' && !h.sync?.ask) return { ok: false, chip: '配置没写进去', at: h.sync?.last?.at };
  if (t && !t.ok) return { ok: false, chip: '自检没过', why: `自检没通过：${t.why ?? '没回话'}`, act: 'selftest', actLabel: '再试一次', at: t.at };
  return { ok: true, at: t?.at };
}

/** 最近一次自检的时间（没做过为 0） */
export const lastTest = (h: HostInfo) => (h.selftest?.items ?? []).reduce((m, i) => Math.max(m, i.at), 0);

export type LineTone = 'ok' | 'busy' | 'bad' | 'muted';
export interface ConfigLine {
  tone: LineTone;
  text: string;
  /** 写入失败：可以重试（重新下发同步请求） */
  retry?: boolean;
}

/** 这个 agent 的配置写进去没有：已写入 <供应商> · <模型> · <强度> · 时间 / 正在写入… / 不改 / 服务器上没给它配 / 机器离线 / 写入失败 */
export function configLine(h: HostInfo, a: HostAgent, view: ProviderView | null, now: number, pick: RunPick = runPick(view, a)): ConfigLine {
  if (pick.mode === 'off') return { tone: 'muted', text: '不改：Board 不碰它的配置文件' };
  if (pick.none) return { tone: 'muted', text: '服务器上没给它配供应商：不写' };
  if (!h.sync?.able) return { tone: 'muted', text: `board ${h.version || '?'} 是旧版：按老办法同步，升级后在这里看结果` };
  const st = h.sync.last?.agents.find(s => s.agent === a.id);
  if (!h.online && (h.sync.ask || !st)) return { tone: 'muted', text: `机器离线：上线后写入 ${runWords(pick)}` };
  if (h.sync.ask) return { tone: 'busy', text: `正在写入 ${runWords(pick)}…` };
  if (!st) return { tone: 'busy', text: '等 board host 写入…' };
  if (st.state === 'failed') return { tone: 'bad', text: `写入失败：${st.why ?? '原因没报上来'}`, retry: true };
  if (st.state === 'unsupported') return { tone: 'muted', text: `不支持：${st.why ?? '这个 agent 不能按服务器上的配置写'}` };
  if (st.state === 'none') return { tone: 'muted', text: '服务器上没给它配供应商：不写' };
  if (st.state === 'off') return { tone: 'muted', text: '不改：Board 不碰它的配置文件' };
  const words = [st.provider, st.model, st.effort ? `强度 ${st.effort}` : st.dropped ? '不写强度' : '强度默认'].filter(Boolean).join(' · ');
  return { tone: 'ok', text: `已写入 ${words} · ${since(h.sync.last!.at, now)}` };
}

/* ------------------------------ skills 和 MCP ------------------------------ */

export interface SyncLine {
  tone: LineTone;
  text: string;
}

/** skills 一行：用服务器的写了几个、写进了哪里 / 正在写入 / 写入失败；不改时只装 board-skills */
export function skillsLine(h: HostInfo, lib: number | null, now: number): SyncLine {
  if (!h.syncSkills) return { tone: 'muted', text: '只装 board-skills，其余不碰' };
  const r = h.sync?.last?.skills;
  const n = lib ?? r?.count ?? 0;
  if (h.sync?.ask && h.online) return { tone: 'busy', text: `正在写入库里的 ${n} 个 skill…` };
  if (!h.online && h.sync?.ask) return { tone: 'muted', text: `机器离线：上线后写入库里的 ${n} 个 skill` };
  if (!r || r.state === 'off' || !r.state) return { tone: 'busy', text: '等 board host 写入…' };
  if (r.state === 'failed') return { tone: 'bad', text: `写入失败：${r.why ?? '原因没报上来'}` };
  return { tone: 'ok', text: `库里的 ${r.count} 个 skill 已写进这台机器各个 agent 的 skill 目录 · ${since(h.sync!.last!.at, now)}` };
}

/** MCP 一行：写进了哪几个 agent */
export function mcpLine(h: HostInfo, names: string[] | null, now: number): SyncLine {
  if (!h.syncMcp) return { tone: 'muted', text: 'Board 不碰这台机器上的 MCP 配置' };
  const r = h.sync?.last?.mcp;
  const n = names?.length ?? r?.count ?? 0;
  if (h.sync?.ask && h.online) return { tone: 'busy', text: `正在写入 ${n} 个 MCP…` };
  if (!h.online && h.sync?.ask) return { tone: 'muted', text: `机器离线：上线后写入 ${n} 个 MCP` };
  if (!r || r.state === 'off' || !r.state) return { tone: 'busy', text: '等 board host 写入…' };
  if (r.state === 'failed') return { tone: 'bad', text: `写入失败：${r.why ?? '原因没报上来'}` };
  const list = names?.length ? `（${names.join('、')}）` : '';
  const where = r.agents.length ? `已写进 ${r.agents.join('、')}` : '已写入（这台机器上没有会用它们的 agent）';
  return { tone: 'ok', text: `库里的 ${r.count} 个 MCP${list}${where} · ${since(h.sync!.last!.at, now)}` };
}

/* ------------------------------ 工作空间 ------------------------------ */

export interface WsException {
  key: string;
  /** 项目 */
  label: string;
  text: string;
  tone: 'info' | 'bad';
  /** 克隆失败：可以重新克隆这块白板（和地址） */
  reclone?: { board: string; url: string };
}

const projectOf = (board: string) => board.split('/')[0];

/** 工作空间里只列例外：--dir 单独指定的（说明）、克隆失败的、不是 git 仓库的（要处理） */
export function workspaceExceptions(h: HostInfo): WsException[] {
  const out: WsException[] = [];
  for (const o of h.workspace?.overrides ?? []) out.push({ key: `o:${o.board}`, label: o.board.includes('/') ? o.board : projectOf(o.board), text: `用 --dir 单独指定：${o.path}`, tone: 'info' });
  const seen = new Set<string>();
  for (const d of h.dirs) {
    if (seen.has(d.path) || !dirFresh(h, d)) continue;
    seen.add(d.path);
    if (d.status === 'not_git') out.push({ key: d.path, label: projectOf(d.board), text: `${d.path} 不是 git 仓库`, tone: 'bad' });
    else if (d.status === 'missing' && d.clone?.state === 'failed')
      out.push({ key: d.path, label: projectOf(d.board), text: `克隆失败：${d.clone.message || '原因没报上来'}`, tone: 'bad', reclone: { board: d.board, url: d.clone.url } });
  }
  return out;
}

/** 还没有目录的项目（第一次派来时建好或克隆；正在克隆、克隆失败的不算） */
export function laterProjects(h: HostInfo): string[] {
  const out: string[] = [];
  for (const d of h.dirs) {
    const p = projectOf(d.board);
    if (d.status === 'missing' && (!d.clone || d.clone.state === 'done') && !out.includes(p)) out.push(p);
  }
  return out;
}

/** 每个项目能不能推送（自检结果）：同一个项目的几块白板是同一个目录，只列一行，有一块没通过就算没通过 */
export function pushRows(h: HostInfo) {
  const out: { key: string; label: string; ok: boolean; why?: string; at: number }[] = [];
  for (const i of (h.selftest?.items ?? []).filter(x => x.kind === 'git')) {
    const p = projectOf(i.key);
    const row = out.find(r => r.key === p);
    if (!row) out.push({ key: p, label: `${p} 能推送`, ok: i.ok, why: i.why ?? undefined, at: i.at });
    else if (row.ok && !i.ok) Object.assign(row, { ok: false, why: i.why ?? undefined, at: i.at });
  }
  return out;
}

/** Windows 用 \，其余用 / */
export const sepOf = (h: Pick<HostInfo, 'os'>) => (/windows/i.test(h.os) ? '\\' : '/');

/* ------------------------------ 概况 ------------------------------ */

/** 今天交审几张、异常退出几次（按本地的「今天」） */
export function todayCounts(recent: Dispatch[] | null, now: number) {
  const d = new Date(now);
  d.setHours(0, 0, 0, 0);
  const list = (recent ?? []).filter(x => (x.endedAt ?? 0) >= d.getTime());
  return { review: list.filter(x => x.result === 'review').length, crashed: list.filter(x => x.result === 'crashed').length };
}
