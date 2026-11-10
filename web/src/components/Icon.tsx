/* 图标：统一 20×20 栅格，描边 1.7 */
import type { ReactNode } from 'react';

const P: Record<string, ReactNode> = {
  plus: <path d="M10 4.5v11M4.5 10h11" />,
  folder: <path d="M3 6.5A1.5 1.5 0 0 1 4.5 5h3.2l1.6 1.8h6.2A1.5 1.5 0 0 1 17 8.3v6.2a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 3 14.5Z" />,
  more: (
    <>
      <circle cx="5" cy="10" r=".9" fill="currentColor" />
      <circle cx="10" cy="10" r=".9" fill="currentColor" />
      <circle cx="15" cy="10" r=".9" fill="currentColor" />
    </>
  ),
  tag: (
    <>
      <path d="M3.5 10.2V4.5a1 1 0 0 1 1-1h5.7l6.3 6.3a1 1 0 0 1 0 1.4l-5.6 5.6a1 1 0 0 1-1.4 0Z" />
      <circle cx="7" cy="7" r="1.1" />
    </>
  ),
  prompt: (
    <>
      <path d="M4 4.5h12v8.5H9.5L6 16v-3H4Z" />
      <path d="m7.2 7.6 1.8 1.4-1.8 1.4M10.5 10.5h2.4" />
    </>
  ),
  board: (
    <>
      <rect x="3" y="3.5" width="14" height="13" rx="2.4" />
      <path d="M7.7 3.5v13M12.3 3.5v13" />
    </>
  ),
  chevUp: <path d="m5.5 12.5 4.5-4.5 4.5 4.5" />,
  chevDown: <path d="m5.5 8 4.5 4.5L14.5 8" />,
  chevRight: <path d="m8 5 5 5-5 5" />,
  chevLeft: <path d="m12 5-5 5 5 5" />,
  close: <path d="m5.5 5.5 9 9M14.5 5.5l-9 9" />,
  clip: <path d="m15.2 9.6-5.4 5.4a3.4 3.4 0 0 1-4.8-4.8l5.9-5.9a2.3 2.3 0 0 1 3.2 3.2l-5.8 5.8a1.1 1.1 0 0 1-1.6-1.6l5.3-5.3" />,
  file: (
    <>
      <path d="M11.5 3.5H6a1.5 1.5 0 0 0-1.5 1.5v10A1.5 1.5 0 0 0 6 16.5h8a1.5 1.5 0 0 0 1.5-1.5V7.5Z" />
      <path d="M11.5 3.5v4h4" />
    </>
  ),
  lock: (
    <>
      <rect x="4.5" y="8.5" width="11" height="8" rx="2.2" />
      <path d="M7 8.5V6.6a3 3 0 0 1 6 0v1.9" />
    </>
  ),
  unlock: (
    <>
      <rect x="4.5" y="8.5" width="11" height="8" rx="2.2" />
      <path d="M7 8.5V6.6a3 3 0 0 1 5.8-1.1" />
    </>
  ),
  trash: <path d="M4 6h12M8 6V4.5h4V6M6 6l.8 10h6.4L14 6" />,
  comment: <path d="M4 5.5A2 2 0 0 1 6 3.5h8a2 2 0 0 1 2 2V12a2 2 0 0 1-2 2H9l-3.5 2.7V14H6a2 2 0 0 1-2-2Z" />,
  send: (
    <>
      <path d="M4 10 16 4.5 12.5 16l-2.3-4.4L4 10Z" />
      <path d="m10.2 11.6 2.6-2.6" />
    </>
  ),
  check: <path d="m4.5 10.5 3.5 3.5 7.5-8" />,
  checkCircle: (
    <>
      <circle cx="10" cy="10" r="6.8" />
      <path d="m7.2 10.2 2 2 3.8-4" />
    </>
  ),
  alert: (
    <>
      <path d="M10 3.3 17.2 16H2.8Z" />
      <path d="M10 8.3v3.4M10 13.9v.1" />
    </>
  ),
  info: (
    <>
      <circle cx="10" cy="10" r="6.8" />
      <path d="M10 9.2v4M10 6.7v.1" />
    </>
  ),
  move: <path d="M4 10h11.5M11.5 6l4 4-4 4" />,
  claim: (
    <>
      <path d="M10 3.5v5M7.4 6.1 10 8.6l2.6-2.5" />
      <rect x="3.5" y="10.5" width="13" height="6" rx="2" />
    </>
  ),
  bolt: <path d="M11 2.5 4.5 11H10l-1 6.5L15.5 9H10l1-6.5Z" />,
  sparkle: <path d="M10 3v3.5M10 13.5V17M3 10h3.5M13.5 10H17M5.2 5.2l2.2 2.2M12.6 12.6l2.2 2.2M5.2 14.8l2.2-2.2M12.6 7.4l2.2-2.2" />,
  timer: (
    <>
      <circle cx="10" cy="11" r="6" />
      <path d="M10 8v3l2 1.4M8 3h4" />
    </>
  ),
  edit: <path d="M12.8 4.2a1.8 1.8 0 0 1 2.6 2.6L7.5 14.7 4 15.7l1-3.5Z" />,
  sort: <path d="M3.5 5.5h8M3.5 10h6M3.5 14.5h4M14.5 4.5v11M12 13l2.5 2.5L17 13" />,
  viewCard: (
    <>
      <rect x="3.5" y="3.5" width="13" height="13" rx="2.5" />
      <path d="M6.5 8h7M6.5 11.5h4.5" />
    </>
  ),
  viewLine: <path d="M3.5 5.5h13M3.5 10h13M3.5 14.5h13" />,
  viewTile: (
    <>
      <rect x="3.5" y="3.5" width="5.5" height="5.5" rx="1.2" />
      <rect x="11" y="3.5" width="5.5" height="5.5" rx="1.2" />
      <rect x="3.5" y="11" width="5.5" height="5.5" rx="1.2" />
      <rect x="11" y="11" width="5.5" height="5.5" rx="1.2" />
    </>
  ),
  reorder: <path d="M7 4.5v11M4.5 7 7 4.5 9.5 7M13 15.5v-11M10.5 13l2.5 2.5 2.5-2.5" />,
  renew: (
    <>
      <path d="M15.5 9A5.5 5.5 0 1 0 14 13.5" />
      <path d="M15.8 4.8V9h-4.2" />
    </>
  ),
  terminal: (
    <>
      <rect x="2.5" y="3.5" width="15" height="13" rx="3" />
      <path d="m6 8 2.5 2L6 12M10.5 12.5h3.5" />
    </>
  ),
  sun: (
    <>
      <circle cx="10" cy="10" r="3.4" />
      <path d="M10 2.5v1.6M10 15.9v1.6M2.5 10h1.6M15.9 10h1.6" />
    </>
  ),
  monitor: (
    <>
      <rect x="2.5" y="4" width="15" height="11" rx="2" />
      <path d="M7 17.5h6" />
    </>
  ),
  inbox: (
    <>
      <path d="M3 11.5h4l1 2h4l1-2h4" />
      <path d="M5 4.5h10l2 7V15a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 3 15v-3.5Z" />
    </>
  ),
  copy: (
    <>
      <rect x="7" y="7" width="9.5" height="9.5" rx="2" />
      <path d="M13 7V5a1.5 1.5 0 0 0-1.5-1.5h-6A1.5 1.5 0 0 0 4 5v6a1.5 1.5 0 0 0 1.5 1.5H7" />
    </>
  ),
  keyboard: (
    <>
      <rect x="2.5" y="5" width="15" height="10" rx="2" />
      <path d="M5.5 8h.1M8.5 8h.1M11.5 8h.1M14.5 8h.1M6.5 12h7" />
    </>
  ),
  search: (
    <>
      <circle cx="9" cy="9" r="5.5" />
      <path d="m13.2 13.2 3.3 3.3" />
    </>
  ),
  filter: <path d="M3.5 5h13M6 10h8M8.5 15h3" />,
  collapse: <path d="m6 8 4-3.5L14 8M6 15.5 10 12l4 3.5" />,
  target: (
    <>
      <circle cx="10" cy="10" r="3" />
      <path d="M10 2.5v3M10 14.5v3M2.5 10h3M14.5 10h3" />
    </>
  ),
  fit: <path d="M3.5 7.5v-4h4M16.5 7.5v-4h-4M3.5 12.5v4h4M16.5 12.5v4h-4" />,
  user: (
    <>
      <circle cx="10" cy="7" r="3.2" />
      <path d="M4 16.5c.9-2.8 3.2-4.3 6-4.3s5.1 1.5 6 4.3" />
    </>
  ),
  key: (
    <>
      <circle cx="13" cy="7" r="3.5" />
      <path d="M10.5 9.5 3.5 16.5M6 14l2 2M8 12l1.5 1.5" />
    </>
  ),
  logout: <path d="M8 4.5H5.5a1.5 1.5 0 0 0-1.5 1.5v8a1.5 1.5 0 0 0 1.5 1.5H8M12 6.5 15.5 10 12 13.5M15.5 10H8" />,
  /* 机器（board host）：两层服务器 */
  host: (
    <>
      <rect x="3" y="3.5" width="14" height="5.5" rx="1.6" />
      <rect x="3" y="11" width="14" height="5.5" rx="1.6" />
      <path d="M6 6.25h.01M6 13.75h.01M9 6.25h5M9 13.75h5" />
    </>
  ),
  /* 派发：把卡片交给机器 */
  dispatch: <path d="M3.5 10h9M9.5 6l4 4-4 4M16.5 4.5v11" />,
  stop: <rect x="5.5" y="5.5" width="9" height="9" rx="1.6" />,
  pause: <path d="M7.5 5.5v9M12.5 5.5v9" />,
  play: <path d="M6.5 4.8v10.4L15 10Z" />,
  log: <path d="M4 5h12M4 8.5h12M4 12h8M4 15.5h5" />,
  /** 齿轮：和活动栏「设置」同一个图形，缩到 20 的画布 */
  settings: (
    <g transform="scale(0.8333)">
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 13.5a7.6 7.6 0 0 0 0-3l2-1.6-2-3.4-2.4.9a7.5 7.5 0 0 0-2.6-1.5L14 2.4h-4l-.4 2.5A7.5 7.5 0 0 0 7 6.4l-2.4-.9-2 3.4 2 1.6a7.6 7.6 0 0 0 0 3l-2 1.6 2 3.4 2.4-.9a7.5 7.5 0 0 0 2.6 1.5l.4 2.5h4l.4-2.5a7.5 7.5 0 0 0 2.6-1.5l2.4.9 2-3.4Z" />
    </g>
  ),
};

export type IconName = keyof typeof P;

export function Icon({ name, sw = 1.7, className }: { name: IconName; sw?: number; className?: string }) {
  return (
    <svg
      viewBox="0 0 20 20"
      fill="none"
      stroke="currentColor"
      strokeWidth={sw}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden="true"
    >
      {P[name]}
    </svg>
  );
}

/* 活动栏 / 新建弹窗里用的 24×24 图标 */
export const BoardIcon24 = () => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinejoin="round" aria-hidden="true">
    <rect x="3.5" y="4" width="17" height="16" rx="3" />
    <path d="M9.2 4v16M14.8 4v16" />
  </svg>
);
export const InboxIcon24 = () => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinejoin="round" strokeLinecap="round" aria-hidden="true">
    <path d="M3.5 13.5h4.8l1.4 2.6h4.6l1.4-2.6h4.8" />
    <path d="M6.2 5h11.6l2.7 8.5v4A2.5 2.5 0 0 1 18 20H6a2.5 2.5 0 0 1-2.5-2.5v-4Z" />
  </svg>
);
export const ProjectsIcon24 = () => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinejoin="round" strokeLinecap="round" aria-hidden="true">
    <rect x="2.5" y="9" width="6.5" height="6" rx="1.8" />
    <rect x="15" y="3.5" width="6.5" height="6" rx="1.8" />
    <rect x="15" y="14.5" width="6.5" height="6" rx="1.8" />
    <path d="M9 12h2.2c1.6 0 1.6-5.5 3.8-5.5M11.2 12c1.6 0 1.6 5.5 3.8 5.5" />
  </svg>
);
export const SettingsIcon24 = () => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinejoin="round" aria-hidden="true">
    <circle cx="12" cy="12" r="3" />
    <path d="M19.4 13.5a7.6 7.6 0 0 0 0-3l2-1.6-2-3.4-2.4.9a7.5 7.5 0 0 0-2.6-1.5L14 2.4h-4l-.4 2.5A7.5 7.5 0 0 0 7 6.4l-2.4-.9-2 3.4 2 1.6a7.6 7.6 0 0 0 0 3l-2 1.6 2 3.4 2.4-.9a7.5 7.5 0 0 0 2.6 1.5l.4 2.5h4l.4-2.5a7.5 7.5 0 0 0 2.6-1.5l2.4.9 2-3.4Z" />
  </svg>
);
/** 机器（board host）：两层服务器 */
export const HostsIcon24 = () => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinejoin="round" strokeLinecap="round" aria-hidden="true">
    <rect x="3.5" y="4" width="17" height="6.5" rx="2" />
    <rect x="3.5" y="13.5" width="17" height="6.5" rx="2" />
    <path d="M7 7.25h.01M7 16.75h.01M11 7.25h6M11 16.75h6" />
  </svg>
);
