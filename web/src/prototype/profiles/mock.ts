/* ==========================================================================
   原型（用完即弃）：开发环境下把 api 里机器、派发、白板、卡片详情换成带假数据的版本
   - 工作区：机器换成 model.ts 里的 4 台（mac、win、win2、mbp），派发是它们在跑的会话；领取规则换成「现在」的写法（规则 → agent），给 ?pg=0 看
   - oss/main：多塞 5 张待办（501–505，带 bug / Windows / 重构 / 文档 / 没标签）和 #512（待审核，动态是线上真实的那 58 条）
   - 只读：派发、改规则都只改内存（model.ts），不发写请求
   ========================================================================== */
import { api } from '../../lib/api';
import { normHost } from '../../lib/normalize';
import { buildVersion } from '../../lib/build';
import type { Activity, BoardSnapshot, Card, CardDetail, ClaimRule, Dispatch, HostAgent, HostInfo, Workspace } from '../../lib/types';
import { ACT_512 } from './act512';
import { agentName, useModel, type AgentId } from './model';

const T = Date.now();
const MIN = 60_000;

/* ------------------------------ 机器（给「现在」的界面看） ------------------------------ */

/** 「现在」每台机器上每个 agent 写进配置文件的模型：同一个 agent 在不同机器上不一样 */
const OLD_MODEL: Record<string, Partial<Record<AgentId, [string, string]>>> = {
  mac: { claude: ['model-a', 'max'], codex: ['model-a', 'max'], dsh: ['model-a', 'max'] },
  win: { claude: ['sonnet[1m]', ''], codex: ['model-b', 'max'], dsh: ['model-b', ''] },
  win2: { claude: ['model-a', 'max'], codex: ['model-a', 'max'], dsh: ['model-a', 'max'] },
  mbp: { claude: ['opus', 'high'], codex: ['model-b', 'high'] },
};

function hostInfos(): HostInfo[] {
  return useModel.getState().hosts.map((h, i) => {
    const agents: HostAgent[] = (['claude', 'codex', 'dsh'] as AgentId[]).map(id => {
      const a = h.agents[id];
      const m = OLD_MODEL[h.name]?.[id];
      return {
        id,
        name: agentName(id),
        version: a ? (id === 'claude' ? '2.1.329 (Claude Code)' : id === 'codex' ? 'codex-cli 0.130.0' : '0.2.2-rc.5') : '',
        found: !!a,
        env: h.os.startsWith('Windows') ? 'git-bash' : 'login',
        model: m?.[0],
        effort: m?.[1] || undefined,
        run_mode: 'off',
      };
    });
    return {
      id: i + 1,
      name: h.name,
      os: h.os,
      version: '0.1.46',
      agents,
      envs: [],
      workspace: { path: h.os.startsWith('Windows') ? 'D:\\work' : '~/work' },
      dirs: [],
      max: h.max,
      perSession: 4,
      paused: false,
      agent: 'codex',
      cronOn: false,
      cron: [],
      round: h.online,
      auto: true,
      windows: h.win ? [h.win] : [[0, 1440]],
      online: h.online,
      firstAt: T - 30 * 24 * 60 * MIN,
      beatAt: h.online ? T - 4000 : T - (h.offMin ?? 0) * MIN,
      offAt: h.online ? null : T - (h.offMin ?? 0) * MIN,
    } as HostInfo;
  });
}

function dispatches(): Dispatch[] {
  const { hosts, profiles } = useModel.getState();
  let id = 9000;
  return hosts.flatMap((h, i) =>
    h.running.map(r => {
      const p = profiles.find(x => x.id === r.profile);
      const [board, card] = r.ref.split('#');
      id += 1;
      return {
        id,
        board,
        card: Number(card),
        title: r.title,
        hostId: i + 1,
        host: h.name,
        agent: p?.agent ?? 'codex',
        agentLabel: agentName(p?.agent ?? 'codex'),
        name: `${h.name}/${p?.model.replace('/', '-') ?? 'x'}${p?.effort ? `-${p.effort}` : ''}`,
        env: 'login',
        envName: '登录 shell',
        source: 'cron',
        by: { kind: 'system', name: 'board' },
        state: 'running',
        result: null,
        createdAt: T - r.min * MIN - 5000,
        startedAt: T - r.min * MIN,
        endedAt: null,
        exitCode: null,
        stopBy: null,
        beatAt: T - 3000,
        session: id,
        seq: 1,
        total: 1,
        model: p?.model ?? null,
        effort: p?.effort ?? null,
        kind: '',
      } as Dispatch;
    }),
  );
}

/** 「现在」的领取规则：规则只能指到 agent */
const OLD_RULES: ClaimRule[] = [
  { op: 'all', labels: ['bug', 'Windows'], agent: 'codex', active: true },
  { op: 'in', labels: ['bug'], agent: 'codex', active: true },
  { op: 'in', labels: ['文档'], agent: 'dsh', active: true },
  { op: 'in', labels: ['重构'], agent: 'claude', active: true },
];

/* ------------------------------ oss/main 多塞的卡 ------------------------------ */

const EXTRA: { id: number; col: string; title: string; labels: string[]; body: string; age: number }[] = [
  { id: 501, col: 'todo', title: 'Windows 安装脚本替换失败时不回滚旧的 board.exe', labels: ['bug', 'Windows'], body: 'gh#171\n\n替换 `board.exe` 失败（被占用）时，旧文件已经被改名，新文件没放上去。', age: 40 },
  { id: 502, col: 'todo', title: '`board ls` 在空白板上报 index out of range', labels: ['bug'], body: 'gh#174', age: 32 },
  { id: 503, col: 'todo', title: '删掉 `--legacy` 参数和它的分支', labels: ['重构'], body: '0.9 起没人用了。', age: 25 },
  { id: 504, col: 'todo', title: 'README 的安装一节按新脚本重写', labels: ['文档'], body: '', age: 18 },
  { id: 505, col: 'todo', title: '统计每周交审张数，写进周报', labels: [], body: '', age: 9 },
  {
    id: 512,
    col: 'review',
    title: '[P1] 修复：批量整理 / 稍后 / 回收站部分失败仍报成功',
    labels: ['bug'],
    body: '（原型：这张卡的动态是线上 default/main#512 的真实记录，打开右边的动态看「调度记录折叠」。）',
    age: 0,
  },
];

let labelIds: Map<string, number> | null = null;
function learnLabels(w: Workspace) {
  labelIds = new Map((w.projects.find(p => p.key === 'oss')?.labels ?? []).map(l => [l.name.toLowerCase(), l.id]));
}
async function ensureLabels() {
  if (!labelIds) learnLabels(await origWorkspace());
  return labelIds!;
}

/** #512 的动态：最后一条在一小时前 */
function activity512(): Activity[] {
  const last = ACT_512[ACT_512.length - 1][0];
  const base = T - (last + 60) * MIN;
  return ACT_512.map(([m, k, action, ak, name, text], i) => ({
    id: 51200 + i,
    at: base + m * MIN,
    actor: ak === 'human' ? { kind: 'human', name: '' } : ak === 'agent' ? { kind: 'agent', name } : { kind: 'system', name: name || 'board' },
    kind: k === 'c' ? 'comment' : 'system',
    action,
    // 线上的截图本地打不开
    text: text.replace(/!\[[^\]]*\]\([^)]*\)/g, '（截图）'),
  })) as Activity[];
}

function cardOf(x: (typeof EXTRA)[number], ids: Map<string, number>): Card {
  const created = x.id === 512 ? T - (ACT_512[ACT_512.length - 1][0] + 60) * MIN : T - x.age * MIN;
  return {
    id: x.id,
    title: x.title,
    body: x.body,
    column: x.col,
    holder: null,
    claimedAt: null,
    beatAt: null,
    runStartedAt: null,
    workMs: x.id === 512 ? 62 * MIN : 0,
    runs: x.id === 512 ? 7 : 0,
    runsLost: x.id === 512 ? 6 : 0,
    claimedFrom: null,
    labels: x.labels.map(l => ids.get(l.toLowerCase())).filter((v): v is number => v != null),
    createdAt: created,
    updatedAt: T - (x.id === 512 ? 60 : x.age) * MIN,
    movedAt: T - (x.id === 512 ? 60 : x.age) * MIN,
    createdBy: x.id === 512 ? { kind: 'agent', name: 'win/model-a-max-1' } : { kind: 'human', name: '' },
    comments: x.id === 512 ? 4 : 0,
  } as Card;
}

function detailOf(x: (typeof EXTRA)[number], ids: Map<string, number>): CardDetail {
  const c = cardOf(x, ids);
  const activity: Activity[] =
    x.id === 512 ? activity512() : [{ id: x.id * 100, at: c.createdAt, actor: { kind: 'human', name: '' }, kind: 'system', action: 'create', text: '新建了卡片' } as Activity];
  return { ...c, activity, attachments: [] };
}

/* ------------------------------ 装上 ------------------------------ */

const origWorkspace = api.workspace;

export function installMock() {
  // 原型的服务端从主检出跑，版本号可能和界面差一个：别弹「服务端已经升级，刷新页面」
  const origInfo = api.info;
  api.info = (async () => ({ ...(await origInfo()), version: buildVersion() })) as typeof api.info;
  api.workspace = (async () => {
    const w = await origWorkspace();
    learnLabels(w);
    return {
      ...w,
      hosts: hostInfos().map(normHost),
      dispatches: dispatches(),
      blocks: [],
      sched: { ...(w.sched ?? { minute: 0 }), rules: OLD_RULES, fallback: null, fallbackActive: false, picks: [] },
    } as Workspace;
  }) as typeof api.workspace;
  api.hosts = async () => ({ hosts: hostInfos().map(normHost), dispatches: dispatches() });

  const origBoard = api.board;
  api.board = (async (path: string, full?: boolean) => {
    const b: BoardSnapshot = await origBoard(path, full);
    if (path !== 'oss/main') return b;
    const ids = await ensureLabels();
    for (const x of EXTRA) {
      b.cards[x.id] = cardOf(x, ids);
      const col = (b.cols[x.col] ??= []);
      if (!col.includes(x.id)) col.push(x.id);
    }
    return b;
  }) as typeof api.board;

  const origCard = api.card;
  api.card = (async (path: string, id: number) => {
    const x = path === 'oss/main' ? EXTRA.find(e => e.id === id) : undefined;
    if (!x) return origCard(path, id);
    return detailOf(x, await ensureLabels());
  }) as typeof api.card;
}
