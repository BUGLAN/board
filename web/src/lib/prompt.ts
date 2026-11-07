/* ==========================================================================
   复制为提示词：只复制一行 skill 调用。
   怎么领取、怎么干活、规则和错误码都写在 board-skills 里（仓库 .agents/skills/board-skills/SKILL.md），
   agent 拿到引用后自己用 board CLI 读取卡片的全部信息。
   ========================================================================== */

export const SKILL = 'board-skills';

/** 一张卡片：/board-skills mini/main#2 */
export const cardPrompt = (path: string, id: number) => `/${SKILL} ${path}#${id}`;

/** 一块白板（领完领取起点为止）：/board-skills mini/main */
export const boardPrompt = (path: string) => `/${SKILL} ${path}`;
