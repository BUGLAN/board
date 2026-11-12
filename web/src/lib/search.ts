/* ==========================================================================
   命令面板里跨白板搜卡片：高亮什么、哪些结果放到「其他白板」一组
   ========================================================================== */
import type { SearchHit } from './types';

/**
 * 高亮用的文字：`#12`、`oss/main#3`、`label:bug` 这些定位写法没有可高亮的正文（它们靠编号、标签找卡片），
 * 返回空串；普通文字原样返回。
 */
export function highlightQuery(q: string): string {
  const s = q.trim();
  if (!s) return '';
  if (s.startsWith('#') || s.toLowerCase().startsWith('label:')) return '';
  // 白板路径 + 编号：oss/main#3
  const hash = s.lastIndexOf('#');
  if (hash > 0 && s.slice(0, hash).includes('/')) return '';
  return s;
}

/**
 * 「其他白板」一组：跨白板搜索结果里，去掉当前白板的（当前白板的卡片已经在上面的「卡片」一组里），
 * 同一张卡片只留一条。服务端已经按相关度、更新时间排好，这里保持顺序。
 */
export function otherHits(hits: SearchHit[], path: string): SearchHit[] {
  const seen = new Set<string>();
  const out: SearchHit[] = [];
  for (const h of hits) {
    if (h.board === path) continue;
    const k = `${h.board}#${h.id}`;
    if (seen.has(k)) continue;
    seen.add(k);
    out.push(h);
  }
  return out;
}
