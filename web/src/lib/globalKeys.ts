/* 全局快捷键（PRD §10.10）：App 挂在 document 上。单独放在这里，测试可以直接派发按键 */
import { closeDetail, useData } from '../store/data';
import { isImeKey } from './util';
import { closePalette, openCreator, openPalette, openSwitcher, overlayOpen, setSide, setTab, TAB_ORDER, useUI } from '../store/ui';

export function globalKey(e: KeyboardEvent) {
  // 输入法组字时的按键（回车上屏、Esc 取消组字）不是快捷键：输入框自己不再拦它们，冒泡到这里也不能关详情（#684）
  if (isImeKey(e)) return;
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
  if (mod && ['1', '2', '3', '4', '5', '6', '7', '8'].includes(e.key)) {
    e.preventDefault();
    // 弹窗、复盘台开着时不在它们下面换页（#684）
    if (!overlayOpen()) setTab(TAB_ORDER[Number(e.key) - 1]);
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
}
