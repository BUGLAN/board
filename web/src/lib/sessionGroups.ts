/* ==========================================================================
   状态块里同一会话的卡框在一起（default/main#289）
   - 同一个会话的卡（正在做的那张和排在它后面的）挨着放：整组放在组里第一张原来的位置，正在做的在前，排着的按会话里的顺序
   - 不在会话里、等机器空位的排队卡（按机器）也放在一起
   - 两张以上才成组（一张不用框）；派发结束、排队的卡开始做，组跟着变
   纯函数：输入这一块的卡片顺序和进行中的派发，输出显示顺序和每张卡属于哪一组
   ========================================================================== */
import type { Dispatch } from './types';

export interface SessGroup {
  key: string;
  /** session：同一个会话；wait：等机器空位的排队卡 */
  kind: 'session' | 'wait';
  host: string;
  agentLabel: string;
  /** 组里的卡，按显示顺序 */
  ids: number[];
}

const active = (d: Dispatch) => d.state === 'queued' || d.state === 'running';

/** 一次派发属于哪一组：排队、而且会话就是自己（不是开会话时排进别人会话的）= 等机器空位 */
function keyOf(d: Dispatch): { key: string; kind: SessGroup['kind'] } {
  if (d.state === 'queued' && (d.session == null || d.session === d.id)) return { key: `w:${d.hostId}`, kind: 'wait' };
  return { key: `s:${d.session ?? d.id}`, kind: 'session' };
}

/** 这一块的显示顺序：成组的卡挨着放；groups 按卡片编号查它在哪一组（只列两张以上的组） */
export function sessionOrder(ids: number[], dispatches: Dispatch[] | undefined, path: string): { ids: number[]; groups: Record<number, SessGroup> } {
  const of = new Map<number, Dispatch>();
  for (const d of dispatches ?? []) if (active(d) && d.board === path && !of.has(d.card)) of.set(d.card, d);
  if (of.size < 2) return { ids, groups: {} };
  const byKey = new Map<string, { g: SessGroup; at: Map<number, number> }>();
  ids.forEach((id, i) => {
    const d = of.get(id);
    if (!d) return;
    const { key, kind } = keyOf(d);
    let e = byKey.get(key);
    if (!e) {
      e = { g: { key, kind, host: d.host, agentLabel: d.agentLabel, ids: [] }, at: new Map() };
      byKey.set(key, e);
    }
    e.g.ids.push(id);
    e.at.set(id, i);
  });
  const groups: Record<number, SessGroup> = {};
  for (const { g, at } of byKey.values()) {
    if (g.ids.length < 2) continue;
    if (g.kind === 'session') {
      // 正在做的在前，排着的按会话里的第几张
      const rank = (id: number) => {
        const d = of.get(id)!;
        return d.state === 'running' ? -1 : (d.seq ?? Number.MAX_SAFE_INTEGER);
      };
      g.ids.sort((a, b) => rank(a) - rank(b) || at.get(a)! - at.get(b)!);
      const run = of.get(g.ids.find(id => of.get(id)!.state === 'running') ?? g.ids[0])!;
      g.host = run.host;
      g.agentLabel = run.agentLabel;
    }
    for (const id of g.ids) groups[id] = g;
  }
  if (!Object.keys(groups).length) return { ids, groups };
  const out: number[] = [];
  const done = new Set<SessGroup>();
  for (const id of ids) {
    const g = groups[id];
    if (!g) out.push(id);
    else if (!done.has(g)) {
      done.add(g);
      out.push(...g.ids);
    }
  }
  return { ids: out, groups };
}

/** 显示顺序里成组的一段：从 start 开始、连着的 n 张都属于同一组（拖动预览插进来的卡会把一段拆开，各段分开框） */
export function groupRuns(ids: number[], groups: Record<number, SessGroup>): { start: number; n: number; g: SessGroup }[] {
  const out: { start: number; n: number; g: SessGroup }[] = [];
  for (let i = 0; i < ids.length; i++) {
    const g = groups[ids[i]];
    if (!g) continue;
    const last = out[out.length - 1];
    if (last && last.g === g && last.start + last.n === i) last.n++;
    else out.push({ start: i, n: 1, g });
  }
  return out.filter(r => r.n >= 2);
}
