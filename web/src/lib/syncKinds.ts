/* ==========================================================================
   同步到机器的几类配置（skills、MCP、Git 凭据）：一处登记（服务端同一份登记是 crates/board-core/src/hosts.rs 的 SYNC_KINDS）
   形状都一样：服务端一份 → 每台机器一个开关（用服务器的 / 不改）→ 一次同步一行结果（状态、数量、原因，加上各自的明细）→ 每台机器一个胶囊
   - 机器详情「设置」页一类一行（hostDetail 的 syncLine、syncNotes，HostSettings 的 SyncBlock），共用配置里每台机器的胶囊和
     Git 凭据卡片上的一行（syncPills），改开关时当帧画成「正在写入」（Saver 的 touchesConfig）都按这里遍历
   - 每类自己的那几句（不改时怎么说、写好了怎么说、明细里换掉了哪些）登记在这里；
     加一类时它自己的存储、下发接口、board host 里怎么写、共用配置里的区块另做
   ========================================================================== */
import { api } from './api';
import type { GitCred, HostInfo, McpServer, SkillInfo, SyncResult } from './types';

/** 每类在服务端的那一份（设置页取来写「库里的 N 个」，Git 凭据看服务端设了没有） */
export interface SyncLibs {
  skills: SkillInfo[];
  mcp: McpServer[];
  git: { cred: GitCred | null };
}
export type SyncKey = keyof SyncLibs;
/** 机器上这一类的开关（HostInfo 的字段，也是改设置时的那一项） */
export type SyncFlag = 'syncSkills' | 'syncMcp' | 'syncGit';

/** 设置页那一行要用的服务端那一份：有哪些（名称）；服务端没设（Git 凭据） */
export interface SyncLib {
  names: string[];
  none?: boolean;
}

/** 那一行下面的补充：一句话，中间可以夹一段路径（<code>） */
export interface SyncNote {
  text: string;
  code?: string;
  tail?: string;
}

export interface SyncKind {
  key: SyncKey;
  /** 设置页那一行的标题 */
  name: string;
  flag: SyncFlag;
  /** 取服务端的那一份（设置页用），放进 SyncLibs 里它那一格 */
  load: () => Promise<Partial<SyncLibs>>;
  /** 从取到的里面拿设置页那一行要用的；还没取到时 null */
  lib: (libs: Partial<SyncLibs>) => SyncLib | null;
  /** 机器详情「设置」页那一行（syncLine） */
  line: {
    /** 不改 */
    off: string;
    /** 收到同步请求、正在写；n 是要写几个（库里的，取到之前按上一次写的） */
    busy: (n: number) => string;
    /** 有同步请求、机器离线 */
    offline: (n: number) => string;
    /** 服务端没设（只有 Git 凭据有这一步） */
    none?: string;
    /** 旧版 board host 不报这一类：「board x.y.z 是旧版：」后面的半句（后来才加的一类才有） */
    old?: string;
    /** 还没报上来 */
    wait: string;
    /** 失败时开头的那个词，后面接「：原因」 */
    failed: string;
    /** 写好了，后面接「 · 多久前」 */
    ok: (r: SyncResult, lib: SyncLib | null) => string;
  };
  /** 写好了以后那一行下面的补充（换掉了本机同名的、软链接没覆盖、没配的仓库） */
  notes: (r: SyncResult) => SyncNote[];
  /** 共用配置里的胶囊：已写入时悬停写进了哪里 */
  where: (r: SyncResult) => string;
  /** 一台机器一行的区块（Git 凭据卡片）上的那几句；失败、还没报上来和设置页那一行一样 */
  row?: {
    off: string;
    offline: (r: SyncResult | undefined) => string;
    asked: string;
    /** 写好了，后面接「 · 多久前」 */
    ok: (r: SyncResult) => string;
  };
}

const list = (xs: string[]) => xs.join('、');

/** 按界面上的顺序 */
export const SYNC_KINDS: SyncKind[] = [
  {
    key: 'skills',
    name: 'skills',
    flag: 'syncSkills',
    load: () => api.skills().then(skills => ({ skills })),
    lib: l => (l.skills ? { names: l.skills.map(s => s.name) } : null),
    line: {
      off: '只装 board-skills，其余不碰',
      busy: n => `正在写入库里的 ${n} 个 skill…`,
      offline: n => `机器离线：上线后写入库里的 ${n} 个 skill`,
      wait: '等 board host 写入…',
      failed: '写入失败',
      ok: r => `库里的 ${r.skills.count} 个 skill 已写进这台机器各个 agent 的 skill 目录`,
    },
    notes: ({ skills: r }) => [
      ...(r.replaced.length ? [{ text: `替换了本机同名的 ${list(r.replaced)}：旧的挪进了 `, code: '~/.board/host/skills-backup/' }] : []),
      ...(r.links.length ? [{ text: `本机的 ${list(r.links)} 是软链接，没有覆盖` }] : []),
    ],
    where: ({ skills: r }) => (r.dirs.length ? `写进了 ${list(r.dirs)}` : '已写入'),
  },
  {
    key: 'mcp',
    name: 'MCP',
    flag: 'syncMcp',
    load: () => api.mcp().then(mcp => ({ mcp })),
    lib: l => (l.mcp ? { names: l.mcp.map(m => m.name) } : null),
    line: {
      off: 'Board 不碰这台机器上的 MCP 配置',
      busy: n => `正在写入 ${n} 个 MCP…`,
      offline: n => `机器离线：上线后写入 ${n} 个 MCP`,
      wait: '等 board host 写入…',
      failed: '写入失败',
      ok: ({ mcp: r }, lib) => {
        const names = lib?.names.length ? `（${list(lib.names)}）` : '';
        const where = r.agents.length ? `已写进 ${list(r.agents)}` : '已写入（这台机器上没有会用它们的 agent）';
        return `库里的 ${r.count} 个 MCP${names}${where}`;
      },
    },
    notes: ({ mcp: r }) => (r.replaced.length ? [{ text: `换掉了本机同名的 ${list(r.replaced)}：原来的值存进了 `, code: '~/.board/host/', tail: ' 下的备份文件' }] : []),
    where: ({ mcp: r }) => (r.agents.length ? `写进了 ${list(r.agents)}` : '已写入'),
  },
  {
    key: 'git',
    name: 'Git 凭据',
    flag: 'syncGit',
    load: () => api.git().then(git => ({ git })),
    lib: l => (l.git ? { names: [], none: l.git.cred === null } : null),
    line: {
      off: '用这台机器自己的 git 凭据，Board 不碰',
      busy: () => '正在同步：配进 Board 管的仓库…',
      offline: () => '机器离线：上线后同步',
      none: '服务端还没设 Git 凭据：在「Agent → Git 凭据」里设',
      old: '升级后才用服务端的 Git 凭据',
      wait: '等 board host 同步…',
      failed: '同步失败',
      ok: ({ git: r }) => (r.repos.length ? `已同步：${r.host} 的令牌配进了 ${r.repos.length} 个仓库` : `已同步：令牌已就绪，还没有远端是 ${r.host} 的仓库`),
    },
    notes: ({ git: r }) => (r.skipped.length ? [{ text: `没配的仓库：${r.skipped.join('；')}` }] : []),
    where: ({ git: r }) => `配进了 ${r.repos.length} 个仓库`,
    row: {
      off: '不同步：用这台机器自己的 git 凭据',
      offline: r => (r?.git.state === 'synced' ? `离线：上次配进了 ${r.git.repos.length} 个仓库，上线后同步` : '离线：上线后同步'),
      asked: '正在同步…',
      ok: ({ git: r }) => `已同步 · ${r.repos.length ? `配进了 ${r.repos.length} 个仓库` : '令牌已就绪，还没有用这个主机的仓库'}`,
    },
  },
];

export const syncKind = (key: SyncKey): SyncKind => SYNC_KINDS.find(k => k.key === key)!;

/** 改一类的开关（用服务器的 / 不改）的那一项设置 */
export const flagPatch = (k: SyncKind, on: boolean) => ({ [k.flag]: on }) as Partial<Record<SyncFlag, boolean>>;

/** board host 是旧版、不报这一类：会按请求同步，最近一次的结果里却没有这一行（后来才加的 Git 凭据） */
export const syncOld = (k: SyncKind, h: HostInfo) => !!h.sync?.able && !!h.sync.last && !h.sync.last[k.key]?.state;
