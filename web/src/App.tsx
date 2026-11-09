/* ==========================================================================
   Board 界面：网页端与桌面端共用
   顶栏 ─ 活动栏 │ 侧栏 │ 主内容区（白板 / 项目画布 / 设置）
   ========================================================================== */
import { useEffect, useLayoutEffect, useRef } from 'react';
import { BoardSkeleton, BoardView } from './components/BoardView';
import { CanvasPage } from './components/Canvas';
import { CardDetail } from './components/CardDetail';
import { Creator } from './components/Creator';
import { DispatchDialog, HostModal, HostsPage } from './components/Hosts';
import { Login } from './components/Login';
import { ConfirmDialog, CtxMenu, FormModal, LabelManager, LabelPicker, Lightbox, Palette, Toasts } from './components/Overlays';
import { SettingsPage } from './components/Settings';
import { Topbar } from './components/Topbar';
import { TitleStrip } from './components/WindowControls';
import { ActivityBar, SidePanel } from './components/Workbench';
import { openBoard } from './lib/actions';
import { installCardInteractions } from './lib/drag';
import { agentColor, agentGlyph, isMobile, reduceMotion } from './lib/util';
import { checkAuth, useAuth } from './store/auth';
import { boardMeta, boot, closeDetail, onEvents, useCurrent, useData } from './store/data';
import {
  closePalette, flash, openCreator, openPalette, openSwitcher, overlayOpen, setSide, setTab, toast, useUI, type Tab,
} from './store/ui';

const OP_LABEL: Record<string, string> = {
  create: '新建', edit: '编辑', move: '移动', reorder: '排序', claim: '领取', renew: '续期', comment: '评论', edit_comment: '改评论',
  timeout: '释放', lost: '异常退出', beat: '心跳', release: '强制释放', label: '标签', delete: '删除', dispatch: '派发',
};

const PAGE: Record<Tab, 'board' | 'canvas' | 'hosts' | 'settings'> = {
  board: 'board', inbox: 'board', projects: 'canvas', hosts: 'hosts', settings: 'settings',
};

/** 侧栏收起 / 展开时主内容跟着平移（只动 transform，不逐帧改宽度）：
    收起：主内容先占满宽度，从右边滑回来；展开：主内容先保持原来的宽度，从左边滑过去，结束后再收窄 */
function useSideSlide(side: boolean) {
  const first = useRef(true);
  useLayoutEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    const main = document.getElementById('main');
    if (!main || reduceMotion() || isMobile()) return;
    const w = parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--side-w')) || 280;
    main.getAnimations().forEach(a => a.cancel());
    const opts = { duration: 420, easing: 'cubic-bezier(.16, 1, .3, 1)' };
    if (!side) {
      main.animate([{ transform: `translateX(${w}px)` }, { transform: 'none' }], opts);
    } else {
      main.style.marginRight = `-${w}px`;
      const a = main.animate([{ transform: `translateX(-${w}px)` }, { transform: 'none' }], opts);
      const done = () => (main.style.marginRight = '');
      a.onfinish = done;
      a.oncancel = done;
    }
  }, [side]);
}

/** 先确认登录：没登录显示登录页，登录后（或本机模式）才打开白板 */
export function App() {
  const state = useAuth(s => s.state);
  const error = useAuth(s => s.error);
  useEffect(() => {
    checkAuth();
  }, []);
  if (state === 'in') return <BoardApp />;
  return (
    <>
      <Atmosphere />
      <TitleStrip />
      {state === 'out' ? (
        <Login />
      ) : (
        <div className="login">
          <Loading error={state === 'error' ? error : null} />
        </div>
      )}
    </>
  );
}

function Atmosphere() {
  return (
    <div className="atmosphere" aria-hidden="true">
      <div className="dots" />
      <div className="glow g1" />
      <div className="glow g2" />
      <div className="glow g3" />
      <div className="grain" />
    </div>
  );
}

function BoardApp() {
  const tab = useUI(s => s.tab);
  const side = useUI(s => s.side);
  const cur = useCurrent();
  const error = useData(s => s.error);
  const ws = useData(s => s.ws);

  useEffect(() => boot(), []);
  useSideSlide(side);
  useEffect(() => installCardInteractions(), []);

  // 通过 CLI、其他窗口或心跳中断释放的修改：卡片短暂高亮，并标出是谁做的
  useEffect(
    () =>
      onEvents(events => {
        const { path, ws } = useData.getState();
        for (const ev of events) {
          if (ev.board === path) {
            const color = ev.actor.kind === 'agent' ? agentColor(ev.actor.name) : ev.actor.kind === 'system' ? 'var(--danger)' : 'var(--brand)';
            const who = ev.actor.kind === 'agent' ? `${agentGlyph(ev.actor.name)} ${ev.actor.name}` : ev.actor.kind === 'system' ? '⏱ 系统' : '另一个窗口';
            if (ev.op !== 'delete') flash(ev.cardId, color, `${who}${ev.via === 'cli' ? ' · CLI' : ''} · ${OP_LABEL[ev.op] || ev.op}`);
            // 心跳中断（异常退出）或没开始心跳：服务端写好的原因直接给人看
            if (ev.op === 'timeout' || ev.op === 'lost')
              toast({
                type: 'info',
                title: ev.op === 'lost' ? `#${ev.cardId} 心跳中断，判定异常退出` : `#${ev.cardId} 领取已释放`,
                text: ev.text,
              });
          } else if (ev.op === 'timeout' || ev.op === 'lost') {
            const m = boardMeta(ws, ev.board);
            if (m)
              toast({
                type: 'info',
                title: `「${m.project.name} / ${m.board.name}」#${ev.cardId} ${ev.op === 'lost' ? '心跳中断' : '领取已释放'}`,
                text: ev.text,
                action: { label: '打开', fn: () => openBoard(ev.board) },
              });
          }
        }
      }),
    [],
  );

  // 全局快捷键（PRD §10.10）
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      const typing = !!t.closest?.('input, textarea, select, [contenteditable]');
      const mod = e.metaKey || e.ctrlKey;
      const k = e.key.toLowerCase();
      if (mod && k === 'k') {
        e.preventDefault();
        if (useUI.getState().palette.open) closePalette();
        else openPalette();
        return;
      }
      if (mod && k === 'b') {
        e.preventDefault();
        setSide(!useUI.getState().side);
        return;
      }
      if (mod && ['1', '2', '3', '4', '5'].includes(e.key)) {
        e.preventDefault();
        setTab((['board', 'projects', 'inbox', 'settings', 'hosts'] as Tab[])[Number(e.key) - 1]);
        return;
      }
      if (e.key === 'Escape') {
        if (overlayOpen()) return;
        if (useData.getState().detailId != null) {
          e.preventDefault();
          closeDetail();
        }
        return;
      }
      if (typing || mod || e.altKey || overlayOpen()) return;
      if (useData.getState().detailId != null && 'ckjl'.includes(k)) return; // 详情里的快捷键
      if (k === 'n') {
        e.preventDefault();
        openCreator('todo');
      } else if (k === 'i') {
        e.preventDefault();
        openCreator('inbox');
      } else if (k === 'p') {
        e.preventDefault();
        openSwitcher();
      } else if (e.key === '/') {
        e.preventDefault();
        openPalette();
      }
    };
    document.addEventListener('keydown', key);
    return () => document.removeEventListener('keydown', key);
  }, []);

  useEffect(() => {
    if (!cur) return;
    document.title = `${cur.board.name} · ${cur.project.name} · Board`;
  }, [cur?.board.name, cur?.project.name]); // eslint-disable-line react-hooks/exhaustive-deps

  // 主题色跟随当前项目的颜色：按钮、选中态、焦点环、开关等所有用 --brand 的地方
  const projectColor = cur?.project.color;
  useEffect(() => applyAccent(projectColor), [projectColor]);

  const page = PAGE[tab];
  return (
    <>
      <Atmosphere />
      <div className="app" data-tab={tab} data-side={side ? 'open' : 'closed'}>
        <Topbar />
        <ActivityBar />
        <SidePanel />
        <main className="main" id="main">
          {/* 工作区还没到：白板页用骨架占位；出错时显示错误（数据层会自动重试） */}
          {!ws && (page === 'board' && !error ? <BoardSkeleton /> : <Loading error={error} />)}
          {ws && (
            <>
              <Page show={page === 'board'}>
                <BoardView />
              </Page>
              <Page show={page === 'canvas'}>
                <CanvasPage />
              </Page>
              <Page show={page === 'hosts'}>
                <HostsPage />
              </Page>
              <Page show={page === 'settings'}>
                <SettingsPage />
              </Page>
            </>
          )}
        </main>
      </div>
      <CardDetail />
      <HostModal />
      <DispatchDialog />
      <Creator />
      <Palette />
      <FormModal />
      <LabelManager />
      <ConfirmDialog />
      <CtxMenu />
      <LabelPicker />
      <Lightbox />
      <Toasts />
    </>
  );
}

/** 把项目颜色设为界面的主题色（--brand 系列）；没有项目时恢复默认的橙色 */
function applyAccent(color: string | undefined) {
  const s = document.documentElement.style;
  const names = ['--project', '--brand', '--brand-2', '--brand-soft', '--brand-fg'];
  const hex = color && /^#[0-9a-f]{6}$/i.test(color) ? color : null;
  if (!hex) {
    names.forEach(n => s.removeProperty(n));
    return;
  }
  s.setProperty('--project', hex);
  s.setProperty('--brand', hex);
  s.setProperty('--brand-2', `color-mix(in srgb, ${hex} 72%, #fff)`);
  s.setProperty('--brand-soft', `color-mix(in srgb, ${hex} 14%, transparent)`);
  s.setProperty('--brand-fg', luminance(hex) > 0.5 ? '#18181A' : '#fff');
}

/** 相对亮度（WCAG） */
function luminance(hex: string) {
  const [r, g, b] = [1, 3, 5].map(i => {
    const v = parseInt(hex.slice(i, i + 2), 16) / 255;
    return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** 主内容区的一页：切换时淡入 */
function Page({ show, children }: { show: boolean; children: React.ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  const was = useRef(show);
  useEffect(() => {
    const el = ref.current?.firstElementChild as HTMLElement | null;
    if (show && !was.current && el) {
      el.classList.remove('page-in');
      void el.offsetWidth;
      el.classList.add('page-in');
    }
    was.current = show;
  }, [show]);
  return (
    <div ref={ref} style={{ display: show ? 'contents' : 'none' }}>
      {children}
    </div>
  );
}

function Loading({ error }: { error: string | null }) {
  return (
    <div className="boot">
      <div className="logo" aria-hidden="true">
        <i style={{ ['--h' as string]: '100%' }} />
        <i style={{ ['--h' as string]: '70%' }} />
        <i style={{ ['--h' as string]: '46%' }} />
        <i style={{ ['--h' as string]: '82%' }} />
      </div>
      {error ? (
        <>
          <b>连不上 Board 服务</b>
          <span>{error}</span>
          <code>board-server</code>
        </>
      ) : (
        <span>正在打开白板…</span>
      )}
    </div>
  );
}
