/* 本机接任务确认框的正文：机器名、白板与目录的逐行列表、免审批运行的风险 */
import { describe, expect, it } from 'vitest';
import { hostConfirmText, hostDirLines, serverConfirmText, shownHostName } from './localHost';

describe('shownHostName', () => {
  it('写了机器名就用它（去掉两头空白）', () => {
    expect(shownHostName('  win  ', 'WIN-DESKTOP')).toBe('win');
  });

  it('没写时用本机主机名，并注明', () => {
    expect(shownHostName('', 'WIN-DESKTOP')).toBe('WIN-DESKTOP（本机主机名）');
    expect(shownHostName('   ', '')).toBe('（本机主机名）');
  });
});

describe('hostDirLines', () => {
  it('一行一个白板目录，跳过空行', () => {
    expect(
      hostDirLines([
        { board: 'default/main', path: 'E:\\repo\\board' },
        { board: '', path: '' },
        { board: ' mini/main ', path: ' E:\\repo\\mini ' },
      ]),
    ).toEqual(['· default/main → E:\\repo\\board', '· mini/main → E:\\repo\\mini']);
  });

  it('只填了一半的行也列出来，写明缺什么', () => {
    expect(hostDirLines([{ board: 'default/main', path: '' }])).toEqual(['· default/main → （没有目录）']);
    expect(hostDirLines([{ board: '', path: '/repo/board' }])).toEqual(['· （没选白板） → /repo/board']);
  });

  it('目录很多时截断，长列表不会把弹窗撑破', () => {
    const many = Array.from({ length: 45 }, (_, i) => ({ board: `p/main`, path: `/repo/${i}` }));
    const lines = hostDirLines(many);
    expect(lines).toHaveLength(41);
    expect(lines[40]).toBe('…还有 5 条');
  });
});

describe('hostConfirmText', () => {
  it('包含机器名、目录和风险说明，段落之间空一行', () => {
    const text = hostConfirmText('win', 'WIN-DESKTOP', [{ board: 'default/main', path: 'E:\\repo\\board' }]);
    const parts = text.split('\n\n');
    expect(parts).toHaveLength(3);
    expect(parts[0]).toBe('机器名：win');
    expect(parts[1]).toBe('白板与目录：\n· default/main → E:\\repo\\board');
    expect(parts[2]).toContain('以免审批模式运行');
    expect(parts[2]).toContain('只在信任这些白板时确认');
  });

  it('没有目录时说明会怎样，不留空行', () => {
    const text = hostConfirmText('', 'WIN-DESKTOP', []);
    expect(text).toContain('WIN-DESKTOP（本机主机名）');
    expect(text).toContain('（还没有白板目录');
    // 每段都只占一行标题 + 内容，没有连续的空行
    expect(text).not.toContain('\n\n\n');
  });
});

describe('serverConfirmText', () => {
  it('写清从哪换到哪、地址写进哪里、本机命令只对新地址开放', () => {
    const text = serverConfirmText('https://a.example.com', 'https://b.example.com', '0.1.19', '~/.board/config.toml', false);
    const parts = text.split('\n\n');
    expect(parts).toHaveLength(2);
    expect(parts[0]).toBe('从 https://a.example.com 换到 https://b.example.com（Board 0.1.19）。');
    expect(parts[1]).toContain('~/.board/config.toml');
    expect(parts[1]).toContain('和 board login 同一项');
    expect(parts[1]).toContain('本机命令只对新地址的页面开放');
    expect(parts[1]).toContain('旧地址的页面调不到');
    // 没在接任务时不提 board host
    expect(text).not.toContain('board host');
  });

  it('本机在跑 board host 时说明会先停掉、按新地址再启动', () => {
    const text = serverConfirmText('http://127.0.0.1:7420', 'https://b.example.com', '', '~/.board/config.toml', true);
    expect(text).not.toContain('（Board '); // 拿不到版本号时不写
    expect(text.split('\n\n')).toHaveLength(3);
    expect(text).toContain('（board host）：切换时先停掉它');
    expect(text).toContain('按新地址重新打开后再启动');
    expect(text).toContain('正在跑的 agent 会结束');
  });
});
