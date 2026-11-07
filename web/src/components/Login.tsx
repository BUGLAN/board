/* ==========================================================================
   登录页：只有一个「用 GitHub 登录」按钮
   GitHub 回调失败时，服务端带着 ?login_error=<原因>&login=<账号> 回到这里
   桌面端：在系统默认浏览器里登录（用上浏览器里已有的 GitHub 登录状态），
   外壳拿到一次性登录码后，这个窗口打开 /auth/desktop?code=… 换取会话
   ========================================================================== */
import { useEffect, useRef, useState } from 'react';
import { canDesktopLogin, desktopLogin, type DesktopLoginError } from '../lib/platform';

const REASONS: Record<string, (login: string | null, message: string | null) => string> = {
  denied: login => `GitHub 账号${login ? ` ${login} ` : ''}没有访问这个 Board 的权限。`,
  state: () => '登录已过期，请重新登录。',
  github: () => '没能从 GitHub 取得账号信息，请稍后再试。',
  cancelled: () => '已取消 GitHub 授权。',
  timeout: () => '等了很久没有等到浏览器里的登录结果，请重新登录。',
  browser: m => `没能打开浏览器${m ? `：${m}` : '。'}`,
};

function reasonText(reason: string, login: string | null, message: string | null = null) {
  return (REASONS[reason] ?? (() => `登录失败，请重试。${message ? `（${message}）` : ''}`))(login, message);
}

function readError() {
  const q = new URLSearchParams(location.search);
  const reason = q.get('login_error');
  if (!reason) return null;
  history.replaceState(null, '', location.pathname);
  return reasonText(reason, q.get('login'));
}

export function GitHubMark() {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true" fill="currentColor">
      <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8Z" />
    </svg>
  );
}

export function Login() {
  const [error, setError] = useState(readError);
  const [waiting, setWaiting] = useState(false);
  const attempt = useRef(0);
  useEffect(() => {
    document.title = '登录 · Board';
  }, []);

  /** 桌面端：在默认浏览器里登录。旧版外壳不支持时，照常在窗口里打开 /auth/github */
  const login = async (e?: React.MouseEvent) => {
    if (!canDesktopLogin) return;
    e?.preventDefault();
    const my = ++attempt.current;
    setError(null);
    setWaiting(true);
    try {
      const code = await desktopLogin();
      location.replace(`/auth/desktop?code=${encodeURIComponent(code)}`);
    } catch (x) {
      if (x === 'unsupported') {
        location.href = '/auth/github';
        return;
      }
      const err = x as DesktopLoginError;
      // 被新的一次登录取代：这个窗口里又点了一次就继续等，别的窗口发起的就停下
      if (err.reason === 'superseded' && my !== attempt.current) return;
      setWaiting(false);
      if (err.reason !== 'superseded') setError(reasonText(err.reason, err.login ?? null, err.message ?? null));
    }
  };

  return (
    <div className="login">
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
        {waiting ? (
          <>
            <div className="btn primary login-btn waiting" role="status">
              <span className="login-spin" aria-hidden="true" />
              在浏览器中完成登录…
            </div>
            <p className="login-hint">
              已在默认浏览器中打开 GitHub 授权页，完成后会自动回到这里。
              <button type="button" className="login-retry" onClick={() => login()}>
                重新打开
              </button>
            </p>
          </>
        ) : (
          <a className="btn primary login-btn" href="/auth/github" onClick={login}>
            <GitHubMark />
            用 GitHub 登录
          </a>
        )}
      </div>
    </div>
  );
}
