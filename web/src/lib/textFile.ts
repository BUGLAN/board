/* ==========================================================================
   文本类附件（default/main#287）：.txt、.log、.md、.json、.diff、源代码……点开在界面里查看，不下载
   - isTextFile：按类型（mime）或扩展名、常见的无扩展名文件名判断
   - readHead：只读文件的前一部分（流式读，读够就停），大日志不会卡住界面
   ========================================================================== */

const EXT = new Set(
  (
    'txt text log out err trace md markdown rst adoc tex json jsonl ndjson json5 yaml yml toml ini cfg conf env properties ' +
    'csv tsv xml svg html htm css scss sass less diff patch sh bash zsh fish ps1 psm1 bat cmd ' +
    'py rb go rs js mjs cjs ts tsx jsx vue svelte java kt kts scala groovy gradle swift m mm c h cc cpp cxx hpp cs fs ' +
    'php pl pm lua r sql graphql gql proto dart ex exs erl elm clj hs ml nim zig sol tf hcl lock srt vtt'
  ).split(' '),
);

const NAMES = new Set(['dockerfile', 'makefile', 'license', 'readme', 'changelog', 'authors', 'codeowners', 'procfile', '.gitignore', '.gitattributes', '.editorconfig', '.env', '.npmrc']);

const TEXT_MIME = /^(text\/|application\/(json|ld\+json|x-ndjson|xml|x-yaml|yaml|toml|javascript|x-javascript|ecmascript|x-sh|x-shellscript|x-patch|x-diff|sql|graphql|x-httpd-php)\b)/;

/** 这个附件能不能当文本看 */
export function isTextFile(name: string, mime?: string | null): boolean {
  if (TEXT_MIME.test((mime || '').toLowerCase())) return true;
  const n = (name.trim().toLowerCase().split(/[\\/]/).pop() ?? '').replace(/[?#].*$/, '');
  if (NAMES.has(n)) return true;
  const i = n.lastIndexOf('.');
  return i > 0 || (i === 0 && n.length > 1) ? EXT.has(n.slice(i + 1)) : false;
}

/** 查看器最多显示多少字节：再多就只显示前面这些，完整的下载 */
export const TEXT_LIMIT = 256 * 1024;

export interface TextHead {
  text: string;
  /** 文件比显示的长 */
  truncated: boolean;
  /** 读到的字节数（截断时就是 limit） */
  bytes: number;
}

/** 只读响应的前 limit 个字节，读够就停（取消剩下的下载）；截断时末尾不完整的字符丢掉 */
export async function readHead(res: Response, limit = TEXT_LIMIT): Promise<TextHead> {
  const dec = new TextDecoder('utf-8');
  const reader = res.body?.getReader();
  if (!reader) {
    const all = new Uint8Array(await res.arrayBuffer());
    const truncated = all.length > limit;
    const part = truncated ? all.subarray(0, limit) : all;
    return { text: truncated ? dec.decode(part, { stream: true }) : dec.decode(part), truncated, bytes: part.length };
  }
  let text = '';
  let bytes = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) return { text: text + dec.decode(), truncated: false, bytes };
    const room = limit - bytes;
    if (value.length > room) {
      text += dec.decode(value.subarray(0, room), { stream: true });
      void reader.cancel().catch(() => {});
      return { text, truncated: true, bytes: limit };
    }
    bytes += value.length;
    text += dec.decode(value, { stream: true });
  }
}
