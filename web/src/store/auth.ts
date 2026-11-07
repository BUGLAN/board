/* ==========================================================================
   登录状态：线上用 GitHub 登录；本机模式（board-server --local）不需要登录
   - 启动时取 /api/me：401 → 登录页
   - 之后任何请求返回 401（会话过期、在别处退出）→ 回到登录页
   ========================================================================== */
import { create } from 'zustand';
import { api, onUnauthorized, type ApiError } from '../lib/api';
import type { Me } from '../lib/types';

interface AuthState {
  state: 'checking' | 'in' | 'out' | 'error';
  me: Me | null;
  error: string | null;
}

export const useAuth = create<AuthState>(() => ({ state: 'checking', me: null, error: null }));

onUnauthorized(() => useAuth.setState({ state: 'out', me: null }));

let retry = 0;
export async function checkAuth() {
  try {
    const me = await api.me();
    retry = 0;
    useAuth.setState({ state: 'in', me, error: null });
  } catch (e) {
    const err = e as ApiError;
    if (err.code === 'NOT_LOGGED_IN' || err.code === 'INVALID_TOKEN') {
      useAuth.setState({ state: 'out', me: null, error: null });
    } else {
      // 连不上服务端：显示原因，稍后重试
      useAuth.setState({ state: 'error', error: err.message });
      setTimeout(checkAuth, Math.min(8000, 1000 * 2 ** retry++));
    }
  }
}

export async function logout() {
  try {
    await api.logout();
  } finally {
    useAuth.setState({ state: 'out', me: null });
  }
}
