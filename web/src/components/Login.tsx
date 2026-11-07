/* ==========================================================================
   登录页：只有一个「用 GitHub 登录」按钮
   GitHub 回调失败时，服务端带着 ?login_error=<原因>&login=<账号> 回到这里
   ========================================================================== */
import { useEffect, useState } from 'react';
import { startWindowDrag } from '../lib/platform';

const REASONS: Record<string, (login: string | null) => string> = {
  denied: login => `GitHub 账号${login ? ` ${login} ` : ''}没有访问这个 Board 的权限。`,
  state: () => '登录已过期，请重新登录。',
  github: () => '没能从 GitHub 取得账号信息，请稍后再试。',
  cancelled: () => '已取消 GitHub 授权。',
};

function readError() {
  const q = new URLSearchParams(location.search);
  const reason = q.get('login_error');
  if (!reason) return null;
  history.replaceState(null, '', location.pathname);
  return (REASONS[reason] ?? (() => '登录失败，请重试。'))(q.get('login'));
}

export function GitHubMark() {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true" fill="currentColor">
      <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8Z" />
    </svg>
  );
}

export function Login() {
  const [error] = useState(readError);
  useEffect(() => {
    document.title = '登录 · Board';
  }, []);
  return (
    <div className="login">
      {/* 桌面端：没有顶栏时，按住页面顶部拖动窗口 */}
      <div className="login-drag" onMouseDown={() => startWindowDrag()} />
      <div className="login-card">
        <div className="logo" aria-hidden="true">
          <i style={{ ['--h' as string]: '100%' }} />
          <i style={{ ['--h' as string]: '70%' }} />
          <i style={{ ['--h' as string]: '46%' }} />
          <i style={{ ['--h' as string]: '82%' }} />
        </div>
        <h1>Board</h1>
        <p>登录后打开你的白板。agent 通过命令行接入，用的是设置里新建的访问令牌。</p>
        {error && (
          <div className="login-err" role="alert">
            {error}
          </div>
        )}
        <a className="btn primary login-btn" href="/auth/github">
          <GitHubMark />
          用 GitHub 登录
        </a>
      </div>
    </div>
  );
}
