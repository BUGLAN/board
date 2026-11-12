/* 白板的代码仓库（PRD §4）：把白板设置里的 git 远端地址变成网页地址，
   评论里的提交号、gh#123 据此变成链接。支持 GitHub、GitLab、Gitea 的常见写法：
   - scp 写法：git@github.com:o/r.git、git@gitlab.example.com:group/sub/r.git
   - URL：https://github.com/o/r.git、http://gitea.example.com/o/r、ssh://git@host:2222/o/r.git
   推不出来时返回 null（界面上不加链接）。 */

export interface RepoWeb {
  /** 仓库首页，例如 https://github.com/o/r */
  home: string
  /** 一个提交的页面，例如 https://github.com/o/r/commit/<sha>（GitLab 是 /-/commit/） */
  commit: (sha: string) => string
  /** 一个 issue 的页面，例如 https://github.com/o/r/issues/12（GitLab 是 /-/issues/） */
  issue: (n: number) => string
  /** 托管方：host 里带 gitlab 时按 GitLab 的路径；其余按 GitHub / Gitea 的路径 */
  kind: 'github' | 'gitlab' | 'gitea'
}

const trimSlash = (s: string) => s.replace(/\/+$/, '')
const stripGit = (s: string) => s.replace(/\.git$/i, '')

/** git 远端地址 → 网页地址；认不出来时返回 null */
export function repoWeb(repo: string | null | undefined): RepoWeb | null {
  const raw = (repo ?? '').trim()
  if (!raw) return null
  let host = ''
  let path = ''
  // scp 写法：user@host:path（host 后面没有斜杠；IPv6 地址里的冒号在方括号里，不会误判）
  const scp = /^(?:[^@/\s]+@)?([^:/\s[\]]+):(.+)$/.exec(raw)
  const url = /^(?:https?|ssh|git):\/\/(?:[^@/\s]+@)?([^:/\s]+)(?::\d+)?\/(.+)$/i.exec(raw)
  if (url) {
    host = url[1]
    path = url[2]
  } else if (/^[a-z][a-z0-9+.-]*:\/\//i.test(raw)) {
    // 其他协议（ftp: 之类）不认
    return null
  } else if (scp) {
    host = scp[1]
    path = scp[2]
  } else {
    // 只有路径（本地仓库、裸目录）：没有网页
    return null
  }
  path = trimSlash(stripGit(path.trim()))
  // 至少要有 owner/repo 两段，路径里不能有空段
  const parts = path.split('/')
  if (host.includes(' ') || parts.length < 2 || parts.some(p => !p || /\s/.test(p))) return null
  const kind: RepoWeb['kind'] = /gitlab/i.test(host) ? 'gitlab' : /github/i.test(host) ? 'github' : 'gitea'
  const home = `https://${host}/${parts.join('/')}`
  const dash = kind === 'gitlab' ? '/-/' : '/'
  return {
    home,
    kind,
    commit: sha => `${home}${dash}commit/${sha}`,
    issue: n => `${home}${dash}issues/${n}`,
  }
}
