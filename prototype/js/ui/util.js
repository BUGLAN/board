/* ==========================================================================
   Board · UI 工具：DOM、图标、头像颜色、时间、Markdown
   ========================================================================== */
(function (B) {
  'use strict';

  const $ = (sel, root = document) => root.querySelector(sel);
  const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];
  const esc = s => String(s ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const html = (strings, ...vals) => strings.reduce((a, s, i) => a + s + (i < vals.length ? vals[i] : ''), '');
  const el = markup => { const t = document.createElement('template'); t.innerHTML = markup.trim(); return t.content.firstElementChild; };
  const reduceMotion = () => document.documentElement.dataset.motion === 'reduced' || window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  const isMac = /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
  const isMobile = () => window.matchMedia('(max-width: 760px)').matches;

  /* ---------- 图标（统一 20×20 栅格，描边 1.7） ---------- */
  const S = (d, extra = '') => `<svg viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" ${extra}>${d}</svg>`;
  const icons = {
    plus: S('<path d="M10 4.5v11M4.5 10h11"/>'),
    folder: S('<path d="M3 6.5A1.5 1.5 0 0 1 4.5 5h3.2l1.6 1.8h6.2A1.5 1.5 0 0 1 17 8.3v6.2a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 3 14.5Z"/>'),
    more: S('<circle cx="5" cy="10" r=".9" fill="currentColor"/><circle cx="10" cy="10" r=".9" fill="currentColor"/><circle cx="15" cy="10" r=".9" fill="currentColor"/>'),
    tag: S('<path d="M3.5 10.2V4.5a1 1 0 0 1 1-1h5.7l6.3 6.3a1 1 0 0 1 0 1.4l-5.6 5.6a1 1 0 0 1-1.4 0Z"/><circle cx="7" cy="7" r="1.1"/>'),
    prompt: S('<path d="M4 4.5h12v8.5H9.5L6 16v-3H4Z"/><path d="m7.2 7.6 1.8 1.4-1.8 1.4M10.5 10.5h2.4"/>'),
    board: S('<rect x="3" y="3.5" width="14" height="13" rx="2.4"/><path d="M7.7 3.5v13M12.3 3.5v13"/>'),
    chevUp: S('<path d="m5.5 12.5 4.5-4.5 4.5 4.5"/>'),
    chevDown: S('<path d="m5.5 8 4.5 4.5L14.5 8"/>'),
    close: S('<path d="m5.5 5.5 9 9M14.5 5.5l-9 9"/>'),
    lock: S('<rect x="4.5" y="8.5" width="11" height="8" rx="2.2"/><path d="M7 8.5V6.6a3 3 0 0 1 6 0v1.9"/>'),
    unlock: S('<rect x="4.5" y="8.5" width="11" height="8" rx="2.2"/><path d="M7 8.5V6.6a3 3 0 0 1 5.8-1.1"/>'),
    trash: S('<path d="M4 6h12M8 6V4.5h4V6M6 6l.8 10h6.4L14 6"/>'),
    comment: S('<path d="M4 5.5A2 2 0 0 1 6 3.5h8a2 2 0 0 1 2 2V12a2 2 0 0 1-2 2H9l-3.5 2.7V14H6a2 2 0 0 1-2-2Z"/>'),
    send: S('<path d="M4 10 16 4.5 12.5 16l-2.3-4.4L4 10Z"/><path d="m10.2 11.6 2.6-2.6"/>'),
    check: S('<path d="m4.5 10.5 3.5 3.5 7.5-8"/>'),
    checkCircle: S('<circle cx="10" cy="10" r="6.8"/><path d="m7.2 10.2 2 2 3.8-4"/>'),
    alert: S('<path d="M10 3.3 17.2 16H2.8Z"/><path d="M10 8.3v3.4M10 13.9v.1"/>'),
    info: S('<circle cx="10" cy="10" r="6.8"/><path d="M10 9.2v4M10 6.7v.1"/>'),
    move: S('<path d="M4 10h11.5M11.5 6l4 4-4 4"/>'),
    claim: S('<path d="M10 3.5v5M7.4 6.1 10 8.6l2.6-2.5"/><rect x="3.5" y="10.5" width="13" height="6" rx="2"/>'),
    sparkle: S('<path d="M10 3v3.5M10 13.5V17M3 10h3.5M13.5 10H17M5.2 5.2l2.2 2.2M12.6 12.6l2.2 2.2M5.2 14.8l2.2-2.2M12.6 7.4l2.2-2.2"/>'),
    timer: S('<circle cx="10" cy="11" r="6"/><path d="M10 8v3l2 1.4M8 3h4"/>'),
    edit: S('<path d="M12.8 4.2a1.8 1.8 0 0 1 2.6 2.6L7.5 14.7 4 15.7l1-3.5Z"/>'),
    reorder: S('<path d="M7 4.5v11M4.5 7 7 4.5 9.5 7M13 15.5v-11M10.5 13l2.5 2.5 2.5-2.5"/>'),
    renew: S('<path d="M15.5 9A5.5 5.5 0 1 0 14 13.5"/><path d="M15.8 4.8V9h-4.2"/>'),
    terminal: S('<rect x="2.5" y="3.5" width="15" height="13" rx="3"/><path d="m6 8 2.5 2L6 12M10.5 12.5h3.5"/>'),
    sun: S('<circle cx="10" cy="10" r="3.4"/><path d="M10 2.5v1.6M10 15.9v1.6M2.5 10h1.6M15.9 10h1.6"/>'),
    monitor: S('<rect x="2.5" y="4" width="15" height="11" rx="2"/><path d="M7 17.5h6"/>'),
    reset: S('<path d="M4.5 9A5.5 5.5 0 1 1 6 13.5"/><path d="M4.2 4.8V9h4.2"/>'),
    inbox: S('<path d="M3 11.5h4l1 2h4l1-2h4"/><path d="M5 4.5h10l2 7V15a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 3 15v-3.5Z"/>'),
    copy: S('<rect x="7" y="7" width="9.5" height="9.5" rx="2"/><path d="M13 7V5a1.5 1.5 0 0 0-1.5-1.5h-6A1.5 1.5 0 0 0 4 5v6a1.5 1.5 0 0 0 1.5 1.5H7"/>'),
    hand: S('<path d="M7 10V4.8a1.3 1.3 0 0 1 2.6 0V9.5M9.6 9V3.8a1.3 1.3 0 0 1 2.6 0V9.5M12.2 9.2V5.3a1.3 1.3 0 0 1 2.6 0v6.2a5.5 5.5 0 0 1-5.5 5.5h-.6a5 5 0 0 1-3.9-1.9L2.9 11.7a1.3 1.3 0 0 1 2-1.7L7 12"/>'),
    keyboard: S('<rect x="2.5" y="5" width="15" height="10" rx="2"/><path d="M5.5 8h.1M8.5 8h.1M11.5 8h.1M14.5 8h.1M6.5 12h7"/>'),
  };

  /* ---------- agent 身份色 ---------- */
  const AGENT_COLORS = ['#FF7A45', '#4DB5FF', '#3DDC97', '#FFB020', '#FF5C8A', '#9B87FF', '#2DD4BF', '#E879F9', '#84CC16'];
  const GREEK = { alpha: 'α', beta: 'β', gamma: 'γ', delta: 'δ', epsilon: 'ε', zeta: 'ζ', eta: 'η', theta: 'θ', kappa: 'κ', lambda: 'λ', sigma: 'σ', omega: 'ω' };
  const FIXED = { alpha: 0, beta: 1, gamma: 2, delta: 3 };
  function agentColor(name) {
    const key = String(name || '').toLowerCase();
    if (key in FIXED) return AGENT_COLORS[FIXED[key]];
    let h = 0; for (const ch of key) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
    return AGENT_COLORS[h % AGENT_COLORS.length];
  }
  const agentGlyph = name => GREEK[String(name).toLowerCase()] || String(name || '?').trim().charAt(0).toUpperCase();
  const avatar = (actor, cls = '') => {
    if (!actor || actor.kind === 'human') return `<span class="avatar human ${cls}" title="你（网页）">你</span>`;
    if (actor.kind === 'system') return `<span class="avatar ${cls}" style="--c:var(--text-3)" title="系统">⏱</span>`;
    return `<span class="avatar ${cls}" style="--c:${agentColor(actor.name)}" title="${esc(actor.name)}">${esc(agentGlyph(actor.name))}</span>`;
  };

  /* ---------- 时间 ---------- */
  const pad2 = n => String(n).padStart(2, '0');
  const hhmm = t => { const d = new Date(t); return `${pad2(d.getHours())}:${pad2(d.getMinutes())}`; };
  const mmss = ms => { ms = Math.max(0, ms); const m = Math.floor(ms / 60000), s = Math.floor(ms / 1000) % 60; return `${pad2(m)}:${pad2(s)}`; };
  function ago(t, now) {
    const d = Math.max(0, now - t);
    if (d < 45e3) return '刚刚';
    if (d < 3600e3) return `${Math.round(d / 60e3)} 分钟前`;
    if (d < 86400e3) return `${Math.round(d / 3600e3)} 小时前`;
    const x = new Date(t); return `${x.getMonth() + 1}月${x.getDate()}日 ${hhmm(t)}`;
  }

  /* ---------- 迷你 Markdown（足够原型展示；正式实现换成 markdown-it 之类） ---------- */
  function inline(s) {
    return esc(s)
      .replace(/`([^`]+)`/g, (_, c) => `<code>${c}</code>`)
      .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
      .replace(/(^|[^*])\*([^*\s][^*]*)\*/g, '$1<em>$2</em>')
      .replace(/\[([^\]]+)\]\((https?:[^)\s]+)\)/g, '<a href="$2" target="_blank" rel="noopener">$1</a>')
      .replace(/(^|[\s(])(https?:\/\/[^\s<)]+)/g, '$1<a href="$2" target="_blank" rel="noopener">$2</a>')
      .replace(/(^|[^\w/])(gh#\d+)/g, '$1<span class="gh">$2</span>');
  }
  function markdown(src) {
    const lines = String(src || '').replace(/\r/g, '').split('\n');
    const out = [];
    let i = 0;
    while (i < lines.length) {
      const l = lines[i];
      if (/^```/.test(l)) {
        const buf = []; i++;
        while (i < lines.length && !/^```/.test(lines[i])) buf.push(lines[i++]);
        i++; out.push(`<pre><code>${esc(buf.join('\n'))}</code></pre>`); continue;
      }
      if (/^#{1,3}\s/.test(l)) { const n = l.match(/^#+/)[0].length; out.push(`<h${n}>${inline(l.slice(n + 1))}</h${n}>`); i++; continue; }
      if (/^>\s?/.test(l)) { const buf = []; while (i < lines.length && /^>\s?/.test(lines[i])) buf.push(lines[i++].replace(/^>\s?/, '')); out.push(`<blockquote>${inline(buf.join(' '))}</blockquote>`); continue; }
      if (/^\s*([-*]|\d+\.)\s/.test(l)) {
        const ordered = /^\s*\d+\./.test(l); const items = [];
        while (i < lines.length && /^\s*([-*]|\d+\.)\s/.test(lines[i])) {
          const t = lines[i++].replace(/^\s*([-*]|\d+\.)\s/, '');
          const m = t.match(/^\[( |x|X)\]\s(.*)$/);
          items.push(m ? `<li class="task ${m[1] !== ' ' ? 'checked' : ''}"><i></i><span>${inline(m[2])}</span></li>` : `<li>${inline(t)}</li>`);
        }
        out.push(ordered ? `<ol>${items.join('')}</ol>` : `<ul>${items.join('')}</ul>`); continue;
      }
      if (!l.trim()) { i++; continue; }
      const buf = [];
      while (i < lines.length && lines[i].trim() && !/^(```|#{1,3}\s|>|\s*([-*]|\d+\.)\s)/.test(lines[i])) buf.push(lines[i++]);
      out.push(`<p>${buf.map(inline).join('<br>')}</p>`);
    }
    return out.join('');
  }

  /* ---------- 环形进度 ---------- */
  const ring = (r, cls = '') => {
    const c = 2 * Math.PI * r, size = (r + 3) * 2;
    return `<svg viewBox="0 0 ${size} ${size}" class="${cls}"><circle class="track" cx="${size / 2}" cy="${size / 2}" r="${r}"/><circle class="bar" cx="${size / 2}" cy="${size / 2}" r="${r}" stroke-dasharray="${c.toFixed(2)}" stroke-dashoffset="0" data-c="${c.toFixed(2)}"/></svg>`;
  };
  const setRing = (svg, p) => { const bar = svg && svg.querySelector('.bar'); if (bar) bar.style.strokeDashoffset = (Number(bar.dataset.c) * (1 - Math.max(0, Math.min(1, p)))).toFixed(2); };

  /* ---------- 轻量事件总线（扩展插件可订阅） ---------- */
  const bus = {
    _m: new Map(),
    on(t, fn) { if (!this._m.has(t)) this._m.set(t, new Set()); this._m.get(t).add(fn); return () => this._m.get(t).delete(fn); },
    emit(t, p) { (this._m.get(t) || []).forEach(fn => { try { fn(p); } catch (e) { console.error(e); } }); },
  };

  function autosize(ta) { ta.style.height = 'auto'; ta.style.height = ta.scrollHeight + 'px'; }

  B.ui = Object.assign(B.ui || {}, {
    $, $$, esc, html, el, icons, agentColor, agentGlyph, avatar, hhmm, mmss, ago, markdown, inline,
    ring, setRing, bus, reduceMotion, isMac, isMobile, autosize,
  });
})(window.Board = window.Board || {});
