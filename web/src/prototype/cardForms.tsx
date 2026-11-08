/* ==========================================================================
   原型（PROTOTYPE，用完即弃，不进 main）：状态块里卡片的三种形态
   问题：已完成越积越多（线上 40+ 张），单行一张一行把块撑得很高，页面底部留出大片空白。
         给每个状态加一个「卡片形态」：卡片 / 单行 / 磁贴（只显示编号，横向排、自动换行），
         看三种形态放在真实白板里的高度、空白和状态信号够不够用。
   在现有白板页上用 ?variant=A|B|C|D 切换预设（底部悬浮条，← → 也可以）：
     A 现状：终点单行，其他卡片（对照组）
     B 已完成磁贴：终点磁贴，其他卡片
     C 全部磁贴：所有状态都用磁贴，压测领取、超时、下一张、筛选这些信号
     D 混搭：可领取状态单行、终点磁贴、其他卡片
   每个状态块标题上有一个黄色虚线的三段小开关，可以单独改这一块的形态（切换变体时复位）。
   数据：终点状态里补 ?fake=N 张假卡片（默认 42，只在内存里，点开没有详情、拖动会报错）；
         变体 C 另在第一个锁定状态补两张被领取的假卡片：一张心跳正常、一张心跳晚了。
   结论（2026-11-08，用户定）：每个状态可选 卡片 / 单行 / 磁贴 三种形态，不再加更多；
         磁贴用紧凑档（?tile=2）：24px 高、按编号位数定宽、间距 4px。宽松档拉伸浪费横向空间，
         极紧档低于 24px 点击下限、信号看不清，且已完成块压到 194px 后页面总高已由其他块决定。
   ========================================================================== */
import { type JSX, useEffect, useLayoutEffect, useMemo, useState, useSyncExternalStore } from 'react';
import type { BoardSnapshot, Card, Label, Status } from '../lib/types';
import { agentColor, cx } from '../lib/util';
import { timing } from '../lib/schema';
import { Avatar, LabelChips, Title, agent } from '../components/bits';
import './cardForms.css';

export type Form = 'card' | 'line' | 'tile';

/* ---------------- 变体开关（只在开发环境） ---------------- */

type Variant = 'A' | 'B' | 'C' | 'D';
const VARIANTS: { key: Variant; name: string; pick: (s: Status) => Form }[] = [
  { key: 'A', name: '现状', pick: s => (s.final ? 'line' : 'card') },
  { key: 'B', name: '已完成磁贴', pick: s => (s.final ? 'tile' : 'card') },
  { key: 'C', name: '全部磁贴', pick: () => 'tile' },
  { key: 'D', name: '混搭', pick: s => (s.final ? 'tile' : s.claim ? 'line' : 'card') },
];
const read = (): Variant | null => {
  if (!import.meta.env.DEV) return null;
  const v = new URLSearchParams(location.search).get('variant');
  return VARIANTS.some(x => x.key === v) ? (v as Variant) : null;
};
let current = read();
// 每块单独改的形态：切换变体时清空
let overrides: Record<string, Form> = {};
const subs = new Set<() => void>();
let ver = 0;
const emit = () => {
  ver++;
  subs.forEach(f => f());
};
function setVariant(v: Variant) {
  const q = new URLSearchParams(location.search);
  q.set('variant', v);
  history.replaceState(null, '', `${location.pathname}?${q}`);
  current = v;
  overrides = {};
  emit();
}
const sub = (f: () => void) => {
  subs.add(f);
  return () => subs.delete(f);
};
export const useVariant = () => useSyncExternalStore(sub, () => current);
const useVer = () => useSyncExternalStore(sub, () => ver);

/* 磁贴密度（?tile=1|2|3）：1 宽松（30px 高、拉伸填满）· 2 紧凑（24px 高、按编号位数定宽）· 3 极紧（20px 高、无边框、不带 #） */
type Density = 1 | 2 | 3;
const DENSITY: { key: Density; name: string }[] = [
  { key: 1, name: '宽松' },
  { key: 2, name: '紧凑' },
  { key: 3, name: '极紧' },
];
let density: Density = (() => {
  const d = Number(new URLSearchParams(location.search).get('tile'));
  return d === 1 || d === 3 ? d : 2;
})();
function setDensity(d: Density) {
  const q = new URLSearchParams(location.search);
  q.set('tile', String(d));
  history.replaceState(null, '', `${location.pathname}?${q}`);
  density = d;
  emit();
}
const useDensity = () => useSyncExternalStore(sub, () => density);

/** 磁贴列表的属性：密度 + 这一块里编号最多几位（紧凑档按位数定宽，同一块里的磁贴一样宽） */
export function useTileList(form: Form | undefined, ids: number[]) {
  const d = useDensity();
  if (form !== 'tile') return {};
  return { 'data-td': d, style: { ['--tile-ch' as string]: String(Math.max(1, ...ids)).length } };
}

/** 这一块用什么形态；没开原型时返回 undefined（走原来的逻辑） */
export function useForm(s: Status): Form | undefined {
  const v = useVariant();
  useVer();
  if (!v) return undefined;
  return overrides[s.key] ?? VARIANTS.find(x => x.key === v)!.pick(s);
}

/* ---------------- 假数据 ---------------- */

const TITLES = [
  '设置页给出按平台的 PATH 命令', '标签区支持拖拽调整顺序', 'CLI 必须配置 BOARD_URL', '删除直连本地库的分支',
  '工作流条拖动排序时的吸附动画', '卡片详情里的附件预览', '侧栏收起状态按 Tab 记忆', '新建卡片支持粘贴图片',
  '「复制为提示词」改成一行 skill 调用', '心跳晚了的卡片在顶栏标黄', 'board doctor 自检 PATH 和令牌', '手机上状态块上下排列',
  '登录页的错误提示', '动态里显示是谁移动了卡片', '项目画布连线的箭头样式', '状态块拖右边缘调宽',
  '修复：拖到锁定状态时没有提示原因', '修复：筛选后计数没有更新', '标签胶囊在深色模式下的对比度', 'Compose 文件补健康检查',
  '首屏骨架和真实布局对齐', '卡片编号用等宽字体', '待审核卡片显示累计耗时', '释放卡片后回到领取起点',
  '修复：快速连点新建会建两张', 'board ls 按优先级输出', '设置页「卡片密度」', '减弱动效时关闭流光边框',
  'Markdown 里的任务清单可以勾选', '修复：长标题撑破卡片', '项目级标签的颜色选择器', 'agent 接入页的错误码表',
  '工作流删除状态时卡片回领取起点', '新卡片弹入动画', '修复：Safari 下拖动卡片闪烁', 'SQLite WAL 定期检查点',
  '令牌轮换', '桌面端窗口记住大小和位置', '通过 CLI 修改时卡片闪光', '卡片评论支持 @agent',
  '修复：时区导致的「x 分钟前」不准', '详情面板的键盘快捷键', 'board flow 查看工作流', '空状态文案按状态角色区分',
];

function fakeBoard(board: BoardSnapshot, n: number, withHeld: boolean, labelIds: number[]): BoardSnapshot {
  const list = board.statuses ?? [];
  const fin = list.find(s => s.final);
  if (!fin) return board;
  const now = Date.now();
  let id = Math.max(0, ...Object.keys(board.cards).map(Number));
  const cards = { ...board.cards };
  const cols = { ...board.cols };
  const mk = (i: number, column: string, extra: Partial<Card> = {}): Card => ({
    id: ++id,
    title: TITLES[i % TITLES.length],
    body: '（原型假数据）',
    column,
    holder: null,
    claimedAt: null,
    beatAt: null,
    runStartedAt: null,
    workMs: (i % 7) * 7 * 60_000,
    runs: i % 3 === 0 ? 0 : 1,
    runsLost: 0,
    claimedFrom: null,
    labels: labelIds.length && i % 3 === 0 ? [labelIds[i % labelIds.length]] : [],
    createdAt: now - (n - i) * 3600_000,
    updatedAt: now - (n - i) * 3600_000,
    createdBy: agent('claude'),
    comments: i % 5 === 0 ? 2 : 0,
    ...extra,
  });
  const done = Array.from({ length: n }, (_, i) => mk(i, fin.key));
  for (const c of done) cards[c.id] = c;
  cols[fin.key] = [...(cols[fin.key] ?? []), ...done.map(c => c.id)];
  const lock = list.find(s => s.lock);
  if (withHeld && lock) {
    const live = mk(3, lock.key, { holder: 'claude-a', claimedAt: now - 20 * 60_000, beatAt: now - 3_000, runStartedAt: now - 19 * 60_000, runs: 1 });
    const late = mk(17, lock.key, { holder: 'deepseek', claimedAt: now - 8 * 60_000, beatAt: now - 32_000, runStartedAt: now - 7 * 60_000, runs: 2, workMs: 12 * 60_000 });
    cards[live.id] = live;
    cards[late.id] = late;
    cols[lock.key] = [...(cols[lock.key] ?? []), live.id, late.id];
  }
  return { ...board, cards, cols };
}

/** 开着原型时给白板补假卡片 */
export function useProtoBoard(board: BoardSnapshot | null, labels: Label[]): BoardSnapshot | null {
  const v = useVariant();
  const n = Number(new URLSearchParams(location.search).get('fake') ?? 42);
  const ids = labels.map(l => l.id).join();
  return useMemo(() => (board && v ? fakeBoard(board, n, v === 'C', ids ? ids.split(',').map(Number) : []) : board), [board, v, n, ids]);
}

/* ---------------- 磁贴的内容 ---------------- */

export function TileBody({ card: c, labels, now, final }: { card: Card; labels: Label[]; now: number; final?: boolean }) {
  const t = timing(c, now);
  return (
    <>
      <span className="tile-id">{c.id}</span>
      {c.holder && <i className={cx('tile-dot', t.beat)} style={{ ['--ac' as string]: agentColor(c.holder) }} />}
      <span className="tile-pop" role="tooltip">
        <span className="tile-pop-head">
          <b>#{c.id}</b>
          {final && <span className="tile-pop-done">已完成</span>}
          {c.holder && (
            <span className={cx('tile-pop-holder', t.beat)}>
              <Avatar actor={agent(c.holder)} />
              {c.holder}
              {t.beat === 'late' && ` · 心跳晚了 ${t.missed} 次`}
            </span>
          )}
        </span>
        <span className="tile-pop-title">{c.title}</span>
        {c.labels.length > 0 && (
          <span className="tile-pop-labels">
            <LabelChips ids={c.labels} labels={labels} />
          </span>
        )}
      </span>
    </>
  );
}

/** 单行形态里非终点的卡片：状态色点 + 编号 + 标题；被领取的在右边显示持有者 */
export function LineBody({ card: c, now }: { card: Card; now: number }) {
  const t = timing(c, now);
  return (
    <div className="card-line pt-line">
      <span className="pt-line-dot" />
      <span className="card-id">#{c.id}</span>
      <span className="pt-line-title">
        <Title text={c.title} />
      </span>
      {c.holder && (
        <span className={cx('pt-line-holder', t.beat)} title={c.holder}>
          <Avatar actor={agent(c.holder)} />
          <i className="beat" />
        </span>
      )}
    </div>
  );
}

/* ---------------- 每块的形态小开关 ---------------- */

const ICONS: Record<Form, JSX.Element> = {
  card: (
    <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5">
      <rect x="2.5" y="2.5" width="11" height="11" rx="2" />
      <path d="M5 6h6M5 9h4" />
    </svg>
  ),
  line: (
    <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5">
      <path d="M2.5 4h11M2.5 8h11M2.5 12h11" />
    </svg>
  ),
  tile: (
    <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5">
      <rect x="2.5" y="2.5" width="4.5" height="4.5" rx="1" />
      <rect x="9" y="2.5" width="4.5" height="4.5" rx="1" />
      <rect x="2.5" y="9" width="4.5" height="4.5" rx="1" />
      <rect x="9" y="9" width="4.5" height="4.5" rx="1" />
    </svg>
  ),
};
const NAMES: Record<Form, string> = { card: '卡片', line: '单行', tile: '磁贴' };

export function FormPicker({ col }: { col: Status }) {
  const form = useForm(col);
  if (!form) return null;
  return (
    <span className="pt-forms" role="radiogroup" aria-label={`${col.name}的卡片形态（原型）`}>
      {(['card', 'line', 'tile'] as Form[]).map(f => (
        <button
          key={f}
          type="button"
          role="radio"
          aria-checked={form === f}
          className={cx(form === f && 'on')}
          data-tip={`${NAMES[f]}（原型）`}
          onClick={() => {
            overrides = { ...overrides, [col.key]: f };
            emit();
          }}
        >
          {ICONS[f]}
        </button>
      ))}
    </span>
  );
}

/* ---------------- 悬浮切换条 + 状态读数 ---------------- */

export function PrototypeSwitcher() {
  const v = useVariant();
  useVer();
  const i = Math.max(0, VARIANTS.findIndex(x => x.key === v));
  const go = (d: number) => setVariant(VARIANTS[(i + d + VARIANTS.length) % VARIANTS.length].key);
  const d = useDensity();
  const [m, setM] = useState<{ total: number; blank: number; done: number; row: number } | null>(null);

  useEffect(() => {
    if (!v) return;
    const key = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      if (t.closest('input, textarea, [contenteditable]')) return;
      if (e.key === 'ArrowLeft') go(-1);
      if (e.key === 'ArrowRight') go(1);
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  });

  // 读数：状态块区域总高、最高块和最矮块底边之差（≈ 滚到底时的空白高度）、终点块高度
  useLayoutEffect(() => {
    if (!v) return;
    const measure = () => {
      const mag = document.querySelector<HTMLElement>('.magnet');
      if (!mag) return setM(null);
      const tiles = [...mag.querySelectorAll<HTMLElement>('[data-tile]')];
      const bottoms = tiles.map(t => {
        const y = new DOMMatrixReadOnly(getComputedStyle(t).transform).m42;
        return y + t.offsetHeight;
      });
      const fin = mag.querySelector<HTMLElement>('.col.is-final');
      const kids = [...(fin?.querySelector('.col-list')?.children ?? [])] as HTMLElement[];
      setM({
        row: kids.filter(k => k.offsetTop === kids[0]?.offsetTop).length,
        total: Math.round(Math.max(0, ...bottoms)),
        blank: Math.round(Math.max(0, ...bottoms) - Math.min(...bottoms)),
        done: fin?.offsetHeight ?? 0,
      });
    };
    const t = setInterval(measure, 500);
    measure();
    return () => clearInterval(t);
  }, [v]);

  if (!v) return null;
  return (
    <div className="pt-switcher" role="toolbar" aria-label="原型变体">
      <button type="button" onClick={() => go(-1)} aria-label="上一个变体">
        ‹
      </button>
      <span>
        原型 · <b>{v}</b> {VARIANTS[i].name}
        {Object.keys(overrides).length > 0 && ' · 已单独改'}
      </span>
      <span className="pt-density" role="radiogroup" aria-label="磁贴密度">
        磁贴
        {DENSITY.map(x => (
          <button key={x.key} type="button" role="radio" aria-checked={d === x.key} className={cx(d === x.key && 'on')} onClick={() => setDensity(x.key)}>
            {x.name}
          </button>
        ))}
      </span>
      {m && (
        <span className="pt-meter">
          终点块 <b>{m.done}</b>px · 每行 <b>{m.row}</b> 个 · 总高 <b>{m.total}</b> · 底部高差 <b>{m.blank}</b>
        </span>
      )}
      <button type="button" onClick={() => go(1)} aria-label="下一个变体">
        ›
      </button>
    </div>
  );
}
