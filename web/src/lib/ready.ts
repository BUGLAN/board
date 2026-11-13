/* 机器就绪清单：一台机器要能接卡需要具备的条件，逐项检查；多机对比视图里每台机器一行、和其他机器不一致的格子标出来。
 *
 * 每一项的数据来自做它的那张卡；那张卡还没做的项先不列（不显示、也不算进 N/M）：
 *   开机自启或已装成服务（#190）、白板的环境检查（#215）、skills / MCP / 环境变量同步（#186、#187、#200）、agent 自检（#191）、
 *   每个 agent 的思考强度（#182）。做了的卡在这里加一项、在对比视图里加一列。 */
import { dirFresh, envName, osKind, since, skillGaps } from './hosts';
import type { HostInfo } from './types';

export type ReadyKey = 'online' | 'version' | 'dirs' | 'skills' | 'agent';
/** 没通过时按钮打开机器详情里的哪一块 */
export type ReadyGo = 'dirs' | 'agents';

export interface ReadyCheck {
  key: ReadyKey;
  /** 清单里的名称 */
  label: string;
  /** 对比视图的列名 */
  short: string;
  ok: boolean;
  /** 没通过的原因 */
  why?: string;
  /** 没通过时的按钮：打开机器详情里对应的设置 */
  go?: ReadyGo;
  act?: string;
  /** 没有按钮能处理时（要在那台机器上操作）写怎么做 */
  hint?: string;
}

/** x.y.z 比大小；有一边解析不出来时返回 null */
export function cmpVersion(a: string, b: string): number | null {
  const parse = (v: string) => {
    const m = /^v?(\d+)\.(\d+)\.(\d+)/.exec(v.trim());
    return m ? m.slice(1).map(Number) : null;
  };
  const x = parse(a);
  const y = parse(b);
  if (!x || !y) return null;
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] - y[i];
  return 0;
}

/** 一台机器的就绪清单。server 是服务端的版本（GET /api/info），还没取到时不列「board 版本」这一项 */
export function readiness(h: HostInfo, now: number, server?: string | null): ReadyCheck[] {
  const out: ReadyCheck[] = [];

  out.push(
    h.online
      ? { key: 'online', label: '在线', short: '在线', ok: true }
      : {
          key: 'online',
          label: '在线',
          short: '在线',
          ok: false,
          why: `离线，最近一次在线是${since(h.offAt ?? h.beatAt, now)}`,
          hint: '在那台机器上启动 board host，或在它的桌面端「设置 → 本机」打开「让这台电脑接任务」',
        },
  );

  if (server) {
    const c = h.version ? cmpVersion(h.version, server) : null;
    if (!h.version || c != null) {
      const ok = c != null && c >= 0;
      out.push({
        key: 'version',
        label: 'board 版本不落后于服务端',
        short: 'board 版本',
        ok,
        ...(!ok && {
          why: h.version ? `board ${h.version}，服务端是 ${server}` : 'board host 没有上报版本',
          hint: `在机器详情「维护」里点「升级到 ${server}」，或在那台机器上执行 board update；桌面端运行的更新桌面端到最新版`,
        }),
      });
    }
  }

  // 有工作空间就视为有这块白板的目录（目录状态只在详情里提示，不影响接卡）；没配时派不了卡
  const ws = h.workspace?.path?.trim() ?? '';
  const bad = h.dirs.filter(d => dirFresh(h, d) && (d.status === 'missing' || d.status === 'not_git'));
  const dirsWhy = !ws
    ? '还没有配工作空间，派不了卡'
    : bad
        .map(d =>
          d.status === 'not_git'
            ? `${d.board} 的目录不是 git 仓库`
            : `${d.board} 的目录不存在${d.clone && (d.clone.state === 'queued' || d.clone.state === 'running') ? '（正在克隆）' : ''}`,
        )
        .join('；');
  out.push({
    key: 'dirs',
    label: '每块白板的目录和仓库正常',
    short: '工作空间',
    ok: !dirsWhy,
    ...(!!dirsWhy && { why: dirsWhy, go: 'dirs' as const, act: ws ? '去处理' : '设置工作空间' }),
  });

  const miss = skillGaps(h).missing;
  out.push({
    key: 'skills',
    label: 'agent 看得到 board-skills',
    short: 'board-skills',
    ok: !miss.length,
    ...(miss.length > 0 && {
      why: `${miss.map(a => a.name).join('、')} 看不到 board-skills，派给${miss.length > 1 ? '它们' : '它'}的卡片会做不成`,
      go: 'agents' as const,
      act: '去安装',
    }),
  });

  const def = h.agent ? h.agents.find(a => a.id === h.agent) : undefined;
  out.push({
    key: 'agent',
    label: '有能用的默认 agent',
    short: '默认 agent',
    ok: !!def?.found,
    ...(!def?.found && {
      why: h.agents.length ? '本机没找到能用的 agent' : 'board host 没有上报 agent',
      go: 'agents' as const,
      act: '看本机 agent',
    }),
  });

  return out;
}

/* ---------- 多机对比 ---------- */

export interface CmpCell {
  text: string;
  /** 就绪项：通过 / 没通过；不是就绪项时不写 */
  ok?: boolean;
  /** 和其他机器不一致 */
  diff?: boolean;
  /** 鼠标放上去看的说明 */
  tip?: string;
}

export interface CmpCol {
  key: string;
  title: string;
  cells: CmpCell[];
}

export interface CmpGroup {
  key: string;
  title: string;
  /** agent 的标识（画图标用）；就绪、board 这两组没有 */
  agent?: string;
  cols: CmpCol[];
}

/** 一列里和其他机器不一致的格子：null 是没有数据（旧版 board host 没上报、这台机器没有这个 agent），不参与比较。
 *  只有一种值时都一致；有一个值占多数时标出其他的；几种值一样多时（例如两台机器各一个版本）全部标出 */
export function oddOnes(values: (string | null)[]): boolean[] {
  const counts = new Map<string, number>();
  for (const v of values) if (v != null) counts.set(v, (counts.get(v) ?? 0) + 1);
  if (counts.size <= 1) return values.map(() => false);
  const top = Math.max(...counts.values());
  const modes = [...counts].filter(([, n]) => n === top).map(([v]) => v);
  const mode = modes.length === 1 ? modes[0] : null;
  return values.map(v => v != null && v !== mode);
}

/** 按组比较：同一组（例如同一种系统）里的值才互相比较 */
function oddBy(values: (string | null)[], group: string[]): boolean[] {
  const out = values.map(() => false);
  for (const g of new Set(group)) {
    const idx = group.map((x, i) => (x === g ? i : -1)).filter(i => i >= 0);
    oddOnes(idx.map(i => values[i])).forEach((d, j) => (out[idx[j]] = d));
  }
  return out;
}

const col = (key: string, title: string, values: (string | null)[], diff: boolean[], tips?: (string | undefined)[]): CmpCol => ({
  key,
  title,
  cells: values.map((v, i) => ({ text: v ?? '—', diff: diff[i] || undefined, tip: tips?.[i] })),
});

/** 对比视图的列：就绪的每一项、board 版本、每个 agent 的版本 / 模型 / 供应商 / 启动环境。hosts 的顺序就是行的顺序 */
export function compareHosts(hosts: HostInfo[], now: number, server?: string | null): CmpGroup[] {
  const checks = hosts.map(h => readiness(h, now, server));
  const keys: ReadyCheck[] = [];
  for (const cs of checks) for (const c of cs) if (!keys.some(k => k.key === c.key)) keys.push(c);
  const ready: CmpGroup = {
    key: 'ready',
    title: '就绪',
    cols: [
      {
        key: 'ready:n',
        title: '通过',
        cells: checks.map(cs => {
          const n = cs.filter(c => c.ok).length;
          return { text: `${n}/${cs.length}`, ok: n === cs.length };
        }),
      },
      ...keys.map(k => ({
        key: `ready:${k.key}`,
        title: k.short,
        cells: checks.map(cs => {
          const c = cs.find(x => x.key === k.key);
          return c ? { text: c.ok ? '通过' : '没通过', ok: c.ok, tip: c.why } : { text: '—' };
        }),
      })),
    ],
  };

  // board 版本：和其他机器不一致的标出来；落后于服务端的已经在就绪里算没通过
  const ver = hosts.map(h => h.version || null);
  const board: CmpGroup = { key: 'board', title: 'board', cols: [col('board:version', '版本', ver, oddOnes(ver))] };

  // 至少在一台机器上找到了的 agent，按第一次出现的顺序
  const ids: { id: string; name: string }[] = [];
  for (const h of hosts) for (const a of h.agents) if (a.found && !ids.some(x => x.id === a.id)) ids.push({ id: a.id, name: a.name });
  const os = hosts.map(h => osKind(h.os));
  const agents = ids.map(({ id, name }): CmpGroup => {
    const of = hosts.map(h => h.agents.find(a => a.id === id));
    // 没上报这个 agent 的机器（旧版 board host）不参与比较；上报了但没找到算「没装」，和装了的不一致
    const version = of.map(a => (a ? (a.found ? a.version || '找到了' : '没找到') : null));
    const model = of.map(a => (a?.found && a.model ? a.model : null));
    const env = hosts.map((h, i) => (of[i]?.found ? envName(h, of[i]!.env) : null));
    // 供应商共用配置写进配置文件的供应商和模型（PRD §10.12）；旧版 board host 没上报时不参与比较
    const prov = hosts.map((h, i) => {
      if (!of[i]?.found || h.providers == null) return null;
      const s = h.providers.find(x => x.agent === id);
      if (!s) return h.online ? '还没写入' : null;
      if (s.state === 'synced') return `${s.provider || '?'} / ${s.model || '?'}`;
      if (s.state === 'local') return '本机自己管';
      if (s.state === 'unsupported') return '不支持';
      return '失败';
    });
    const provTip = hosts.map((h, i) => (of[i]?.found ? h.providers?.find(x => x.agent === id)?.why ?? undefined : undefined));
    return {
      key: `agent:${id}`,
      title: name,
      agent: id,
      cols: [
        col(`${id}:version`, '版本', version, oddOnes(version)),
        col(`${id}:model`, '模型', model, oddOnes(model)),
        col(`${id}:provider`, '供应商 / 模型', prov, oddOnes(prov), provTip),
        // 启动环境只在同一种系统的机器之间比（Windows 的 Git Bash 和 macOS 的登录 shell 不算不一致）
        col(`${id}:env`, '启动环境', env, oddBy(env, os)),
      ],
    };
  });

  return [ready, board, ...agents];
}
