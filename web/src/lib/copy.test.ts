import { describe, expect, it } from 'vitest';
import { card, detail } from '../../test/fixtures';
import { carryLabels, filesToCopy, isVersionLabel, pastOf, withNotes } from './copy';
import type { Activity, Attachment, CardDetail, Label } from './types';

const act = (id: number, kind: 'human' | 'agent' | 'system', text: string, comment = true): Activity => ({
  id,
  at: id,
  actor: { kind, name: kind === 'human' ? '' : `${kind}-1` },
  kind: comment ? 'comment' : 'system',
  action: comment ? 'comment' : 'move',
  text,
});
const file = (id: number, name: string): Attachment =>
  ({ id, cardId: 5, name, mime: 'image/png', size: 1, image: true, url: `/api/files/${id}`, markdown: '', createdAt: 0, createdBy: { kind: 'human', name: '' } }) as Attachment;

const src = (): CardDetail => ({
  ...detail(card(5, { runs: 3, runsLost: 2, body: '要做的\n\n![图](/api/files/7)' })),
  activity: [
    act(1, 'system', '派发到 mac · Codex，排队', false),
    act(2, 'system', 'board host（mac）：这次派发结束时……\n\n- 怎么结束的：机器和服务端断开超过 50 秒\n\n未完成的改动：wip/main/5-mac'),
    act(3, 'agent', 'agent 异常退出（退出码 1）\n\n```\n[SYSTEM_ERROR] …\n```'),
    act(4, 'agent', '做法：先结束 board host 再替换。日志 [run.log](/api/files/8)\n\n未完成的改动：`wip/main/5`'),
    act(5, 'human', '不要直接结束 board host。\n换个做法 ![截图](/api/files/9)'),
    act(6, 'agent', '再来一次\n\n未完成的改动：wip/main/5'),
  ],
  attachments: [file(7, 'shot.png'), file(8, 'run.log'), file(9, 'note.png')],
});

describe('复制为新卡片', () => {
  it('原卡的经历：领取、异常退出、agent 和 board host 写的报错、wip 分支（去重）、动态条数、人的评论', () => {
    const p = pastOf(src());
    expect([p.runs, p.lost, p.errors, p.events]).toEqual([3, 2, 2, 6]);
    expect(p.wips).toEqual(['wip/main/5-mac', 'wip/main/5']);
    expect(p.human.map(a => a.id)).toEqual([5]);
  });

  it('默认带过去的标签去掉版本标签，按项目里的顺序', () => {
    const labels = [
      { id: 3, name: 'Windows' },
      { id: 1, name: 'bug' },
      { id: 9, name: 'v0.4.2' },
      { id: 4, name: 'ui' },
    ] as Label[];
    expect(carryLabels([1, 9, 3], labels)).toEqual([3, 1]);
    expect(['v0.1.2', 'V2', ' v1 ', 'vue', 'dev1'].map(isVersionLabel)).toEqual([true, true, true, false, false]);
  });

  it('人的评论附在描述最后一节，空行也引用；没有评论时描述不变', () => {
    const d = src();
    expect(withNotes(d.body, 5, [])).toBe(d.body);
    expect(withNotes(`${d.body}\n`, 5, [act(5, 'human', '第一行\n\n第三行')])).toBe(
      '要做的\n\n![图](/api/files/7)\n\n## 原卡 #5 里人的意见\n\n> 第一行\n>\n> 第三行',
    );
  });

  it('复制描述里引用的原卡附件；只在评论里的不复制，人的评论附进描述后它引用的也复制', () => {
    const d = src();
    expect(filesToCopy(d, d.body).map(a => a.name)).toEqual(['shot.png']);
    expect(filesToCopy(d, withNotes(d.body, 5, pastOf(d).human)).map(a => a.name)).toEqual(['shot.png', 'note.png']);
    expect(filesToCopy(d, '![](/api/files/70)')).toEqual([]);
  });
});
