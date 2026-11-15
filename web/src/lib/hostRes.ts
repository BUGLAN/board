/* ==========================================================================
   机器资源（PRD §10.12「资源」）：机器详情「设置」页最上面四块磁贴（CPU、内存、GPU、存储）要画的东西
   - 取不到的写「不支持」/「未上报」，不画 0；盘、内存用到 90% 起是警告色；快满的盘算进「设置 · N」
   ========================================================================== */
import type { HostInfo, HostRes, ResDisk } from './types';

/** 到这个使用率（%）起用警告色 */
export const RES_WARN = 90;

const GB = 1024 ** 3;

/** 931 GB / 21.4 GB / 1.8 TB */
export function bytes(b: number) {
  const g = b / GB;
  if (g >= 1000) return `${(g / 1024).toFixed(1)} TB`;
  if (g >= 100) return `${Math.round(g)} GB`;
  return `${g.toFixed(1).replace(/\.0$/, '')} GB`;
}

export const pctOf = (used: number, total: number) => (total > 0 ? Math.min(100, Math.round((Math.min(used, total) / total) * 100)) : 0);

export const diskPct = (d: ResDisk) => pctOf(d.total - d.avail, d.total);

/** 快满的盘有几块：算进机器详情页签「设置 · N」 */
export const fullDisks = (h: HostInfo) => h.res?.disks?.filter(d => diskPct(d) >= RES_WARN).length ?? 0;

/** 一块磁贴：大数字（0–100；取不到时是一句话）、趋势、细节 */
export interface Tile {
  key: 'cpu' | 'mem' | 'gpu' | 'disk';
  title: string;
  pct: number | null;
  /** pct 为空时大数字的位置写这个 */
  none?: string;
  sub: string;
  warn: boolean;
  hist: number[];
  /** 第二块起的 GPU、工作空间以外的盘 */
  more: { key: string; label: string; pct: number | null; sub?: string; warn: boolean }[];
  /** 存储那块：大数字是哪块盘、是不是工作空间所在的 */
  disk?: { mount: string; ws: boolean };
}

const load = (l: [number, number, number]) => `负载 ${l.map(x => (x >= 10 ? x.toFixed(0) : x.toFixed(1))).join(' / ')}`;

/** 一块盘写成什么：macOS 的根卷写卷名（Macintosh HD），别的写挂载点 */
export const diskName = (d: ResDisk) => (d.mount === '/' && d.label ? d.label : d.mount);

export function tiles(r: HostRes): Tile[] {
  const c = r.cpu;
  const cpu: Tile = c
    ? { key: 'cpu', title: 'CPU', pct: c.pct, sub: [`${c.cores} 核`, c.load && load(c.load), c.model].filter(Boolean).join(' · '), warn: false, hist: r.hist.cpu, more: [] }
    : { key: 'cpu', title: 'CPU', pct: null, none: '未上报', sub: '这台机器取不到 CPU 使用率', warn: false, hist: [], more: [] };
  const m = r.mem;
  const mp = m ? pctOf(m.used, m.total) : null;
  const mem: Tile = m
    ? { key: 'mem', title: '内存', pct: mp, sub: `${bytes(m.used)} / ${bytes(m.total)}`, warn: (mp ?? 0) >= RES_WARN, hist: r.hist.mem, more: [] }
    : { key: 'mem', title: '内存', pct: null, none: '未上报', sub: '这台机器取不到内存', warn: false, hist: [], more: [] };
  const gs = r.gpus;
  let gpu: Tile;
  if (!gs) gpu = { key: 'gpu', title: 'GPU', pct: null, none: '不支持', sub: '这台机器读不到 GPU', warn: false, hist: [], more: [] };
  else if (!gs.length) gpu = { key: 'gpu', title: 'GPU', pct: null, none: '没有', sub: '没有找到 GPU', warn: false, hist: [], more: [] };
  else {
    const g = gs[0];
    const vram = g.vramTotal ? `显存 ${bytes(g.vramUsed ?? 0)} / ${bytes(g.vramTotal)}` : /^apple/i.test(g.vendor ?? g.name) ? '和内存共用' : '';
    gpu = {
      key: 'gpu',
      title: gs.length > 1 ? `GPU · ${gs.length} 块` : 'GPU',
      pct: g.pct ?? null,
      none: '未上报',
      sub: [g.name || 'GPU', vram].filter(Boolean).join(' · '),
      warn: false,
      hist: g.pct != null ? r.hist.gpu : [],
      more: gs.slice(1).map((x, i) => ({ key: `gpu${i + 1}`, label: `GPU ${i + 2}`, pct: x.pct ?? null, sub: x.name, warn: false })),
    };
  }
  const ds = r.disks;
  let disk: Tile;
  if (!ds?.length) disk = { key: 'disk', title: '存储', pct: null, none: '未上报', sub: '这台机器取不到磁盘', warn: false, hist: [], more: [] };
  else {
    const [d, ...rest] = ds;
    const p = diskPct(d);
    disk = {
      key: 'disk',
      title: '存储',
      pct: p,
      sub: `剩 ${bytes(d.avail)}，共 ${bytes(d.total)}`,
      warn: p >= RES_WARN,
      hist: [],
      disk: { mount: diskName(d), ws: !!d.ws },
      more: rest.map((x, i) => ({ key: `disk${i + 1}`, label: diskName(x), pct: diskPct(x), sub: `剩 ${bytes(x.avail)}`, warn: diskPct(x) >= RES_WARN })),
    };
  }
  return [cpu, mem, gpu, disk];
}

/** 资源块标题旁那一句，没有资源时整块只写的那一句 */
export function resLine(h: HostInfo, now: number, server: string | null, behind: boolean, since: (t: number, now: number) => string): string {
  const r = h.res;
  if (r) return h.online ? `${since(r.at, now)}采样 · 每 10 秒更新` : `离线前最后一次 · ${since(r.at, now)}`;
  if (behind) return `board ${h.version || '?'} 不上报资源${server ? `：升级到 ${server} 后显示` : ''}`;
  if (!h.online) return '机器离线，还没上报过资源';
  return '还没采到资源：下一次上报带上';
}

/** 趋势线：最近几次的使用率画成 0–100 的折线（W×H 的画布，上下各留 2） */
export function sparkPath(data: number[], W = 100, H = 28): { line: string; area: string } | null {
  if (data.length < 2) return null;
  const pts = data.map((v, i) => [(i / (data.length - 1)) * W, H - 2 - (Math.max(0, Math.min(100, v)) / 100) * (H - 4)] as const);
  const line = pts.map(([x, y], i) => `${i ? 'L' : 'M'}${x.toFixed(1)},${y.toFixed(1)}`).join('');
  return { line, area: `${line}L${W},${H}L0,${H}Z` };
}
