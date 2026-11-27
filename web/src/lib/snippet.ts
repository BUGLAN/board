/** 描述的第一行文字：跳过 Markdown 小标题（只有小标题时才用它），去掉列表、引用、勾选框的符号，图片和链接只留文字 */
export function snippet(body: string) {
  const lines = body.split('\n').map(l => l.trim()).filter(Boolean);
  const text = (l: string) => l.replace(/!?\[([^\]]*)\]\([^)]*\)/g, '$1').replace(/^[-*#>\s]+|\[[ xX]\]\s*/g, '').trim();
  return lines.filter(l => !/^#{1,6}\s/.test(l)).map(text).find(Boolean) ?? lines.map(text).find(Boolean) ?? '';
}
