/* ==========================================================================
   Git 凭据（共用配置，PRD §10.12）：服务端存一份 https 远端的令牌，机器选了「用服务器的」就写进用户级 git 配置（按主机全局生效）
   - 表单在界面上先校验（gitCredError），服务端还会再校验、规范化主机（去掉 https://、路径，转小写）
   ========================================================================== */
import type { GitCred } from './types';

export interface GitDraft {
  host: string;
  username: string;
  /** 修改时留空 = 保留原来的 */
  token: string;
  /** 提交作者的用户名；和提交邮箱都填或都空 */
  commitName: string;
  /** 提交作者的邮箱；和提交用户名都填或都空 */
  commitEmail: string;
}

export const blankGit = (): GitDraft => ({ host: 'github.com', username: '', token: '', commitName: '', commitEmail: '' });

export const gitDraftOf = (c: GitCred): GitDraft => ({ host: c.host, username: c.username, token: '', commitName: c.commitName ?? '', commitEmail: c.commitEmail ?? '' });

/** 表单里填的主机：去掉 https://、路径、结尾的 /，转小写（和服务端一样） */
export const normGitHost = (v: string) =>
  v
    .trim()
    .replace(/^https:\/\//i, '')
    .split('/')[0]
    .split('@')
    .pop()!
    .toLowerCase();

/** 没填对的原因；填对了是 null。修改时令牌可以留空（保留原来的） */
export function gitCredError(d: GitDraft, editing: boolean): string | null {
  const raw = d.host.trim();
  if (!raw) return '先填主机，例如 github.com';
  if (/^(ssh:\/\/|git@)|^[^/]*@[^/]*:/i.test(raw) || /^http:\/\//i.test(raw)) return '只支持 https 远端：主机写 github.com 这样的主机名';
  if (!/^[a-z0-9.-]+(:\d{1,5})?$/.test(normGitHost(raw))) return '主机只写主机名，例如 github.com（可以带 :端口）';
  if (!d.username.trim()) return '先填用户名';
  if (/\s/.test(d.username.trim())) return '用户名里不能有空格';
  const commitName = d.commitName.trim();
  const commitEmail = d.commitEmail.trim();
  if (!!commitName !== !!commitEmail) return '提交用户名和提交邮箱要一起填，或都留空';
  if (/[\x00-\x1f\x7f]/.test(d.commitName)) return '提交用户名里不能有换行';
  if (commitEmail) {
    const [name, domain, ...more] = commitEmail.split('@');
    if (!name || !domain || more.length || /[\s\x00-\x1f\x7f]/.test(commitEmail)) return '提交邮箱要填邮箱地址，例如 name@example.com';
  }
  if (!d.token.trim() && !editing) return '先填令牌';
  if (/\s/.test(d.token.trim())) return '令牌里不能有空格、换行';
  return null;
}
