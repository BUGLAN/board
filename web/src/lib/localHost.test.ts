/* 本机接任务确认框的正文：机器名、工作空间、免审批运行的风险 */
import { describe, expect, it } from 'vitest';
import { hostConfirmText, serverConfirmText, shownHostName, workspaceDir } from './localHost';

describe('shownHostName', () => {
  it('写了机器名就用它（去掉两头空白）', () => {
    expect(shownHostName('  win  ', 'WIN-DESKTOP')).toBe('win');
  });

  it('没写时用本机主机名，并注明', () => {
    expect(shownHostName('', 'WIN-DESKTOP')).toBe('WIN-DESKTOP（本机主机名）');
    expect(shownHostName('   ', '')).toBe('（本机主机名）');
  });
});

describe('workspaceDir', () => {
  it('每块白板的目录是 <工作空间>/<项目>/<白板>，分隔符跟着机器走', () => {
    expect(workspaceDir('~/repo', 'default/main')).toBe('~/repo/default/main');
    expect(workspaceDir('C:\\repo\\', 'default/main', 'Windows 11')).toBe('C:\\repo\\default\\main');
    expect(workspaceDir('/srv/repo/', 'p/b', 'macOS 15.4')).toBe('/srv/repo/p/b');
  });

  it('没填工作空间时为空', () => {
    expect(workspaceDir('  ', 'default/main')).toBe('');
    expect(workspaceDir('/repo', '')).toBe('');
  });
});

describe('hostConfirmText', () => {
  it('包含机器名、工作空间和风险说明，段落之间空一行', () => {
    const text = hostConfirmText('win', 'WIN-DESKTOP', 'E:\\repo');
    const parts = text.split('\n\n');
    expect(parts).toHaveLength(3);
    expect(parts[0]).toBe('机器名：win');
    expect(parts[1]).toBe('工作空间：E:\\repo（每块白板的目录是 E:\\repo/<项目>/<白板>）');
    expect(parts[2]).toContain('以免审批模式运行');
    expect(parts[2]).toContain('只在信任这些白板时确认');
  });

  it('没填工作空间时说明会怎样，不留空行', () => {
    const text = hostConfirmText('', 'WIN-DESKTOP', '   ');
    expect(text).toContain('WIN-DESKTOP（本机主机名）');
    expect(text).toContain('（还没填：这台电脑不接任何白板的卡）');
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
