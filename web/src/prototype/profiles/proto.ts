/* ==========================================================================
   原型（用完即弃，不进 main）：「工种」分发——把「用哪个模型」从机器配置里拿出来，派发时选
   问题：调度看不懂、按功能 / 按模型分发难。原型回答「界面长什么样」：
     1. Agent → 自动领取 → 分工（替换「领取规则」）  ?pg=0|A|B|C
        0 = 现在（真组件：规则 → agent）  A 流向图（规则 → 工种 → 机器，三栏连线）
        B 工种卡片（一个工种一张：领哪些卡、在哪跑、现在、成绩单）  C 路由表 + 「这张卡会怎么走」
     2. 卡片详情的「派发」对话框  ?dl=0|X|Y
        0 = 现在（先选机器再选 agent）  X 先选工种，机器默认任意  Y 工种 × 机器一张表
     3. 卡片详情多一段「接下来」（会交给谁、为什么、能不能马上开始）：一直开着
        白板卡片上也写去向  ?chip=1|0
     4. 卡片动态里的调度记录折叠成一行摘要  ?fold=1|0（打开 #512 看，那是线上真实的动态）
   工种、规则、机器都是假数据（mock.ts），在内存里改，刷新回到初始；不发任何写请求
   ========================================================================== */
import { create } from 'zustand';

export type PgKey = '0' | 'A' | 'B' | 'C';
export type DlKey = '0' | 'X' | 'Y';
export const PG: { key: PgKey; name: string }[] = [
  { key: '0', name: '现在' },
  { key: 'A', name: '流向图' },
  { key: 'B', name: '工种卡片' },
  { key: 'C', name: '路由表 + 试一张' },
];
export const DL: { key: DlKey; name: string }[] = [
  { key: '0', name: '现在' },
  { key: 'X', name: '先选工种' },
  { key: 'Y', name: '工种 × 机器' },
];

/** 打开页面时的地址参数：路由会把地址改写成 /项目/白板?tab=…，原型的参数要在那之前记下 */
const initial = new URL(location.href).searchParams;
/** 地址里带了 ?pg=：Agent tab 直接落在「分工」这一节 */
export const startOnPage = initial.has('pg');
const pick = <K extends string>(param: string, list: { key: K }[], store: string, def: K): K => {
  const v = initial.get(param)?.toUpperCase();
  if (v && list.some(x => x.key === v)) return v as K;
  try {
    const s = localStorage.getItem(store);
    if (s && list.some(x => x.key === s)) return s as K;
  } catch {
    /* 没有本地存储 */
  }
  return def;
};
const flag = (param: string, store: string, def: boolean) => {
  const v = initial.get(param);
  if (v === '1' || v === '0') return v === '1';
  try {
    const s = localStorage.getItem(store);
    if (s === '1' || s === '0') return s === '1';
  } catch {
    /* 没有本地存储 */
  }
  return def;
};

export const useProto = create<{ pg: PgKey; dl: DlKey; chip: boolean; fold: boolean }>(() => ({
  pg: pick('pg', PG, 'proto.pf.pg', 'A'),
  dl: pick('dl', DL, 'proto.pf.dl', 'X'),
  chip: flag('chip', 'proto.pf.chip', true),
  fold: flag('fold', 'proto.pf.fold', true),
}));

const remember = (param: string, store: string, v: string) => {
  try {
    localStorage.setItem(store, v);
  } catch {
    /* 没有本地存储 */
  }
  const u = new URL(location.href);
  u.searchParams.set(param, v);
  history.replaceState(history.state, '', u);
};
export const setPg = (pg: PgKey) => {
  useProto.setState({ pg });
  remember('pg', 'proto.pf.pg', pg);
};
export const setDl = (dl: DlKey) => {
  useProto.setState({ dl });
  remember('dl', 'proto.pf.dl', dl);
};
export const setChip = (chip: boolean) => {
  useProto.setState({ chip });
  remember('chip', 'proto.pf.chip', chip ? '1' : '0');
};
export const setFold = (fold: boolean) => {
  useProto.setState({ fold });
  remember('fold', 'proto.pf.fold', fold ? '1' : '0');
};

export const isProto = import.meta.env.DEV;
export const usePg = () => useProto(s => (isProto ? s.pg : '0'));
export const useDl = () => useProto(s => (isProto ? s.dl : '0'));
export const useChip = () => useProto(s => isProto && s.chip);
export const useFold = () => useProto(s => isProto && s.fold);
