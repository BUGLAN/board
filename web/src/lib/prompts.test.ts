import { describe, expect, it } from 'vitest';
import { addPrompt, insertPromptText, listPrompts, readPrompts, removePrompt, savePrompts, updatePrompt, type Prompt } from './prompts';

const row = (id: string, title: string, text: string): Prompt => ({ id, title, text });

describe('快捷提示词（default/main#413）', () => {
  it('新增放在最前面，标题去掉首尾空白、内容保留多行，id 接着已有的往后排', () => {
    const list = [row('p-2', '旧的一条', '第一行\n第二行')];
    const next = addPrompt(list, '  等发版  ', '等发版\r\n发完通知我');
    expect(next.map(p => p.id)).toEqual(['p-3', 'p-2']);
    expect(next[0]).toEqual({ id: 'p-3', title: '等发版', text: '等发版\n发完通知我' });
    expect(addPrompt([], '第一条', '内容')[0].id).toBe('p-1');
  });

  it('标题或内容为空时不新增，原来的列表不变', () => {
    const list = [row('p-1', '等发版', '等发版')];
    expect(addPrompt(list, '   ', '内容')).toEqual(list);
    expect(addPrompt(list, '标题', ' \n ')).toEqual(list);
  });

  it('修改只动目标那一条，id 不在时原样返回', () => {
    const list = [row('p-1', '等发版', '等发版'), row('p-2', '重启', '重启一下')];
    const next = updatePrompt(list, 'p-2', '  重启服务  ', '重启服务\n再试');
    expect(next).toEqual([row('p-1', '等发版', '等发版'), { id: 'p-2', title: '重启服务', text: '重启服务\n再试' }]);
    expect(next).not.toBe(list);
    expect(updatePrompt(list, 'p-9', '新标题', '新内容')).toEqual(list);
    expect(updatePrompt(list, 'p-1', '', '内容')).toEqual(list);
  });

  it('删除只删目标那一条，id 不在时原样返回', () => {
    const list = [row('p-1', '等发版', '等发版'), row('p-2', '重启', '重启一下')];
    expect(removePrompt(list, 'p-1')).toEqual([row('p-2', '重启', '重启一下')]);
    expect(removePrompt(list, 'p-9')).toEqual(list);
  });

  it('本机存的坏数据丢掉，不让缺字段、重复 id 的条目把浮层带坏', () => {
    expect(readPrompts(null)).toEqual([]);
    expect(
      readPrompts([
        null,
        3,
        { id: '', title: '空 id', text: '内容' },
        { id: 'p-1', title: '  ', text: '内容' },
        { id: 'p-1', title: '标题', text: '   ' },
        row('p-1', '留下的', '第一行'),
        row('p-1', '重复的', '第二行'),
        { id: 'p-2', title: '换行', text: 'a\r\nb' },
      ]),
    ).toEqual([row('p-1', '留下的', '第一行'), { id: 'p-2', title: '换行', text: 'a\nb' }]);
  });

  it('保存后刷新还能读回来', () => {
    savePrompts([row('p-1', '等发版', '等发版\n发完通知我')]);
    expect(listPrompts()).toEqual([row('p-1', '等发版', '等发版\n发完通知我')]);
  });

  it('插入到光标处、替换选中文字，没焦点或越界时追加到末尾，光标总在插入内容之后', () => {
    expect(insertPromptText('开头结尾', 2, 2, '等发版')).toEqual({ value: '开头等发版结尾', caret: 5 });
    expect(insertPromptText('开头留空结尾', 2, 4, '等发版')).toEqual({ value: '开头等发版结尾', caret: 5 });
    expect(insertPromptText('开头结尾', null, null, '等发版')).toEqual({ value: '开头结尾等发版', caret: 7 });
    expect(insertPromptText('开头结尾', 99, 120, '等发版')).toEqual({ value: '开头结尾等发版', caret: 7 });
    expect(insertPromptText('开头结尾', 4, 2, '等发版')).toEqual({ value: '开头结尾等发版', caret: 7 });
  });
});
