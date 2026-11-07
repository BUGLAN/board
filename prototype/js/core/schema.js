/* ==========================================================================
   Board · Schema
   与 PRD 对应的常量：列、领取时长、错误码。界面文案集中在这里，便于后续 i18n。
   ========================================================================== */
(function (B) {
  'use strict';

  const COLUMNS = [
    { key: 'todo',   name: '待办',   desc: '等着被做 · board claim 从最上面取' },
    { key: 'doing',  name: '进行中', desc: '被某个 agent 领取，正在做' },
    { key: 'review', name: '待审核', desc: '做完了，等人检查' },
    { key: 'done',   name: '已完成', desc: '人确认完成' },
  ];
  const COLUMN_KEYS = COLUMNS.map(c => c.key);

  /** 「未整理」：放在「待办」之前、还没整理成卡片的条目。不在白板上显示，agent 也不会领取 */
  const INBOX = { key: 'inbox', name: '未整理', desc: '还没整理的计划和想法；整理好后移到「待办」' };
  const ALL_KEYS = [INBOX.key, ...COLUMN_KEYS];
  const COLUMN_NAME = Object.fromEntries([INBOX, ...COLUMNS].map(c => [c.key, c.name]));

  /** 领取时长：30 分钟（PRD §6.1） */
  const CLAIM_MS = 30 * 60 * 1000;

  /** 错误码（PRD §9）+ 默认原因 */
  const ERRORS = {
    NO_IDENTITY:      'CLI 没有提供 agent 名字：使用 --as <名字> 或设置环境变量 BOARD_AS',
    NO_BOARD:         'CLI 没有指定白板：使用 --board <项目>/<白板> 或设置环境变量 BOARD',
    BOARD_NOT_FOUND:  '白板不存在，用 board boards 查看可用的白板',
    NOT_FOUND:        '卡片不存在',
    EMPTY_TITLE:      '标题不能为空',
    EMPTY_TODO:       '「待办」里没有可领取的卡片',
    NOT_IN_TODO:      '这张卡片不在「待办」，也不是你持有的卡片',
    ALREADY_HOLDING:  '你已经持有另一张卡片，一个 agent 同时只能持有一张',
    NOT_HOLDER:       '「进行中」的卡片只有持有者能编辑和移动',
    FORBIDDEN_COLUMN: '不允许移入这一列',
    USAGE:            '命令用法不正确',
  };

  class BoardError extends Error {
    constructor(code, message, detail) {
      super(message || ERRORS[code] || code);
      this.code = code;
      this.detail = detail || null;
    }
  }

  const HUMAN = Object.freeze({ kind: 'human', name: '你' });
  const SYSTEM = Object.freeze({ kind: 'system', name: '系统' });
  const agent = name => ({ kind: 'agent', name });

  B.schema = { COLUMNS, COLUMN_KEYS, INBOX, ALL_KEYS, COLUMN_NAME, CLAIM_MS, ERRORS, BoardError, HUMAN, SYSTEM, agent };
})(window.Board = window.Board || {});
