/* ==========================================================================
   复制为提示词：只复制一行 skill 调用。
   怎么领取、怎么干活、规则和错误码都写在 board-skills 里（仓库 .agents/skills/board-skills/SKILL.md），
   agent 拿到引用后自己用 board CLI 读取卡片的全部信息。
   ========================================================================== */

export const SKILL = 'board-skills';

/** 一张卡片：/board-skills mini/main#2 */
export const cardPrompt = (path: string, id: number) => `/${SKILL} ${path}#${id}`;

/** 一块白板（领完可领取状态为止）：/board-skills mini/main */
export const boardPrompt = (path: string) => `/${SKILL} ${path}`;

/* ---------------- 快捷提示词：一句完整的话，带项目和白板 ----------------
   SKILL.md 的「快捷提示词」一节按这几种写法识别，改句式时两边一起改 */

export type QuickKind = 'split' | 'upload' | 'all' | 'label';

export interface QuickPrompt {
  kind: QuickKind;
  /** 菜单里的名称 */
  title: string;
  /** 复制的内容 */
  text: string;
  /** 复制后的提示：需要接着写什么 */
  hint: string;
}

/** 「default项目之中的main白板」：用标识，agent 直接拼出 BOARD=default/main */
const where = (path: string) => {
  const [project, board] = path.split('/');
  return `${project}项目之中的${board}白板`;
};

/** home：领取起点的名称（默认「待办」） */
export function quickPrompts(path: string, home: string, label?: string): QuickPrompt[] {
  const w = where(path);
  const list: QuickPrompt[] = [
    {
      kind: 'split',
      title: '拆解任务',
      text: `/${SKILL} 把下面的需求拆解成可以独立完成的任务，记到${w}的${home}里：`,
      hint: '粘贴后在冒号后面写上需求',
    },
    {
      kind: 'upload',
      title: '上传任务',
      text: `/${SKILL} 把我们刚才讨论出的任务上传到${w}的${home}里，每个任务一张卡片`,
      hint: '发给刚讨论完任务的 agent',
    },
    {
      kind: 'all',
      title: `做${home}里的全部任务`,
      text: `/${SKILL} 开始做${w}中${home}里的所有任务`,
      hint: '发给装了 board-skills 的 agent 即可',
    },
  ];
  if (label)
    list.push({
      kind: 'label',
      title: `做${home}里带「${label}」的任务`,
      text: `/${SKILL} 开始做${w}中${home}里所有标签带${label}的任务`,
      hint: '发给装了 board-skills 的 agent 即可',
    });
  return list;
}
