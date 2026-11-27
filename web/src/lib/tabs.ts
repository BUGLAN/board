/** 活动栏 Tab：界面、地址栏、数字快捷键共用这一份。 */
export type Tab = 'board' | 'projects' | 'inbox' | 'milestones' | 'stats' | 'hosts' | 'agent' | 'settings';

/** 活动栏从上到下的顺序，也是数字快捷键 ⌘/Ctrl 1~8 的顺序（设置固定在活动栏底部） */
export const TAB_ORDER: Tab[] = ['board', 'inbox', 'milestones', 'stats', 'projects', 'hosts', 'agent', 'settings'];

export const isTab = (t: unknown): t is Tab => TAB_ORDER.includes(t as Tab);
