/* 白板的仓库地址 → 网页地址（PRD §4）：GitHub、GitLab、Gitea 的常见写法；推不出来时不加链接 */
import { describe, expect, it } from 'vitest';
import { repoWeb } from './repo';

describe('repoWeb', () => {
  it('GitHub：scp 写法、https、ssh 地址都认', () => {
    for (const repo of ['git@github.com:o/r.git', 'https://github.com/o/r.git', 'https://github.com/o/r', 'ssh://git@github.com/o/r.git', 'git://github.com/o/r.git']) {
      const w = repoWeb(repo)!;
      expect(w.home, repo).toBe('https://github.com/o/r');
      expect(w.kind).toBe('github');
      expect(w.commit('abc1234')).toBe('https://github.com/o/r/commit/abc1234');
      expect(w.issue(12)).toBe('https://github.com/o/r/issues/12');
    }
  });

  it('GitLab：分组下的仓库、提交和 issue 带 /-/', () => {
    const w = repoWeb('git@gitlab.com:group/sub/repo.git')!;
    expect(w.home).toBe('https://gitlab.com/group/sub/repo');
    expect(w.kind).toBe('gitlab');
    expect(w.commit('abc1234')).toBe('https://gitlab.com/group/sub/repo/-/commit/abc1234');
    expect(w.issue(7)).toBe('https://gitlab.com/group/sub/repo/-/issues/7');
  });

  it('自建 Gitea：ssh 写了端口、https 带端口都认，按 GitHub 的路径拼', () => {
    const a = repoWeb('ssh://git@gitea.example.com:2222/o/r.git')!;
    expect(a.home).toBe('https://gitea.example.com/o/r');
    expect(a.kind).toBe('gitea');
    expect(a.commit('abc1234')).toBe('https://gitea.example.com/o/r/commit/abc1234');
    expect(repoWeb('https://gitea.example.com:3000/o/r.git')!.home).toBe('https://gitea.example.com/o/r');
    expect(repoWeb('git@gitea.example.com:o/r.git')!.home).toBe('https://gitea.example.com/o/r');
  });

  it('推不出来时返回 null：空、本地路径、只有一段路径、其他协议', () => {
    for (const repo of ['', '  ', null, undefined, '/srv/git/r.git', 'C:\\repo\\board', 'https://github.com/o', 'ftp://github.com/o/r.git', 'https://github.com/o/r x']) {
      expect(repoWeb(repo as string | null | undefined), String(repo)).toBeNull();
    }
  });
});
