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

/** 一块白板（领完可领取状态为止）：/board-skills mini/main */
export const boardPrompt = (path: string) => `/${SKILL} ${path}`;
