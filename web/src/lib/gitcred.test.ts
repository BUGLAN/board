import { describe, expect, it } from 'vitest';
import { gitCredError, normGitHost } from './gitcred';

describe('Git 凭据表单', () => {
  it('主机规范化：去掉 https://、路径，转小写', () => {
    expect(normGitHost(' https://GitHub.com/me/board.git ')).toBe('github.com');
    expect(normGitHost('git.example.com:8443')).toBe('git.example.com:8443');
  });
  it('校验：主机只能是 https 的主机名；新设时要填令牌，修改时可以留空', () => {
    const ok = { host: 'github.com', username: 'buglan', token: 'ghp_x' };
    expect(gitCredError(ok, false)).toBeNull();
    expect(gitCredError({ ...ok, host: '' }, false)).toMatch(/先填主机/);
    expect(gitCredError({ ...ok, host: 'git@github.com:me/x.git' }, false)).toMatch(/https/);
    expect(gitCredError({ ...ok, host: 'http://github.com' }, false)).toMatch(/https/);
    expect(gitCredError({ ...ok, host: 'github .com' }, false)).toMatch(/主机名/);
    expect(gitCredError({ ...ok, username: ' ' }, false)).toMatch(/用户名/);
    expect(gitCredError({ ...ok, token: '' }, false)).toMatch(/令牌/);
    expect(gitCredError({ ...ok, token: '' }, true)).toBeNull();
    expect(gitCredError({ ...ok, token: 'a b' }, true)).toMatch(/空格/);
  });
});
