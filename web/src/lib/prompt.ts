/* ==========================================================================
   复制为提示词：只复制一句完整的 skill 调用。
   怎么领取、怎么干活、规则和错误码都写在 board-skills 里（仓库 .agents/skills/board-skills/SKILL.md），
   agent 拿到这句话后自己用 board CLI 读取卡片的全部信息。
   ========================================================================== */

export const SKILL = 'board-skills';

/** 一张卡片：/board-skills 开始执行 mini项目之中的main白板中的#2任务 */
export const cardPrompt = (path: string, id: number) => {
  const [project, board] = path.split('/');
  return `/${SKILL} 开始执行 ${project}项目之中的${board}白板中的#${id}任务`;
};

/**
 * 「未整理」里的条目：/board-skills 拆解 mini项目之中的main白板中的#2需求（拆出的卡记回未整理、挂在这条下面）。
 * force = 界面上判过这一条够细、人点了「强制拆解」（default/main#306）：agent 再判一遍，只在真有先后依赖时才切
 */
export const splitPrompt = (path: string, id: number, { force = false } = {}) => {
  const [project, board] = path.split('/');
  return `/${SKILL} ${force ? '强制' : ''}拆解 ${project}项目之中的${board}白板中的#${id}需求`;
};

/** 卡片详情「复制为提示词」：在「未整理」里的复制拆解这一句（判过够细的是「强制拆解」），其他复制开始执行（PRD §10.9） */
export const detailPrompt = (path: string, c: { id: number; column: string }, force = false) => (c.column === 'inbox' ? splitPrompt(path, c.id, { force }) : cardPrompt(path, c.id));

/** 一块白板（领完可领取状态为止）：/board-skills mini/main */
export const boardPrompt = (path: string) => `/${SKILL} ${path}`;

/* ---------------- 快捷提示词：一句完整的话，带项目和白板 ----------------
   SKILL.md 的「快捷提示词」一节按这几种写法识别，改句式时两边一起改 */

export type QuickKind = 'split' | 'upload' | 'split-inbox' | 'upload-inbox' | 'all' | 'label';

export interface QuickPrompt {
  kind: QuickKind;
  /** 同一次生成里唯一的 id（菜单按键、键盘项都用它做 key） */
  id: string;
  /** 菜单里的名称 */
  title: string;
  /** 复制的内容 */
  text: string;
  /** 复制后的提示：需要接着写什么 */
  hint: string;
  /** 这条针对的可领取状态（all / label 才有） */
  status?: string;
  /** 这条针对的标签（label 才有） */
  label?: string;
}

/** 「default项目之中的main白板」：用标识，agent 直接拼出 BOARD=default/main */
const where = (path: string) => {
  const [project, board] = path.split('/');
  return `${project}项目之中的${board}白板`;
};

/**
 * 拆解的提示词：先判粒度（default/main#306）。需求本身是一个独立的小任务时不拆，整条建一张卡；
 * 偏大才拆，拆出来的每一条都是能单独做完、单独复盘的卡，不按句子数切
 */
const splitText = (target: string) =>
  `/${SKILL} 先判断下面这条需求的粒度：已经是一个独立的小任务就整条建成一张卡，不拆；确实偏大才拆成可以独立完成的任务。` +
  `拆成这样的一张卡只算一次复盘，拆出来的每一条都要能单独做完、单独复盘，不要按句子数切；记到${target}：`;

/**
 * claimables：可领取状态的名称，按工作流顺序。第一个就是新卡片的去向，也是「拆解 / 上传」的领取起点；
 * 每个可领取状态各生成一条「做<状态名>里的全部任务」；传了 label 时每个可领取状态再各生成一条
 * 「做<状态名>里带<标签>的任务」。状态改名或增删时菜单跟着变。
 */
export function quickPrompts(path: string, claimables: string[], label?: string): QuickPrompt[] {
  const w = where(path);
  const names = claimables.length ? claimables : ['待办'];
  const home = names[0];
  const list: QuickPrompt[] = [
    {
      kind: 'split',
      id: 'split',
      title: '拆解任务',
      text: splitText(`${w}的${home}`),
      hint: '粘贴后在冒号后面写上需求',
    },
    {
      kind: 'upload',
      id: 'upload',
      title: '上传任务',
      text: `/${SKILL} 把我们刚才讨论出的任务上传到${w}的${home}里，每个任务一张卡片`,
      hint: '发给刚讨论完任务的 agent',
    },
    {
      kind: 'split-inbox',
      id: 'split-inbox',
      title: '拆解任务到未整理',
      text: splitText(`${w}的未整理`),
      hint: '粘贴后在冒号后面写上需求',
    },
    {
      kind: 'upload-inbox',
      id: 'upload-inbox',
      title: '上传任务到未整理',
      text: `/${SKILL} 把我们刚才讨论出的任务上传到${w}的未整理里，每个任务一张卡片`,
      hint: '发给刚讨论完任务的 agent',
    },
    ...names.map((name, i) => ({
      kind: 'all' as const,
      id: `all-${i}`,
      status: name,
      title: `做「${name}」里的全部任务`,
      text: `/${SKILL} 开始做${w}中${name}里的所有任务`,
      hint: '发给装了 board-skills 的 agent 即可',
    })),
  ];
  if (label)
    names.forEach((name, i) =>
      list.push({
        kind: 'label',
        id: `label-${i}`,
        status: name,
        label,
        title: `做「${name}」里带「${label}」的任务`,
        text: `/${SKILL} 开始做${w}中${name}里所有标签带${label}的任务`,
        hint: '发给装了 board-skills 的 agent 即可',
      }),
    );
  return list;
}
