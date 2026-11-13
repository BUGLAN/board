/* 文本类附件（default/main#287）：哪些点开查看，大文件只读前一部分 */
import { describe, expect, it } from 'vitest';
import { isTextFile, readHead } from './textFile';

describe('isTextFile', () => {
  it('按扩展名：日志、文档、配置、补丁、源代码', () => {
    for (const n of ['prd80-review.txt', 'run.log', 'NOTES.md', 'a.json', 'fix.diff', 'x.patch', 'data.csv', 'main.rs', 'App.tsx', 'deploy.sh', 'conf.YAML'])
      expect(isTextFile(n), n).toBe(true);
  });

  it('按类型：text/*、JSON、XML 这类', () => {
    expect(isTextFile('blob', 'text/plain')).toBe(true);
    expect(isTextFile('blob', 'application/json')).toBe(true);
    expect(isTextFile('blob', 'application/xml; charset=utf-8')).toBe(true);
  });

  it('没有扩展名的常见文本文件', () => {
    expect(isTextFile('Dockerfile')).toBe(true);
    expect(isTextFile('.gitignore')).toBe(true);
    expect(isTextFile('dir/Makefile')).toBe(true);
  });

  it('图片、压缩包、二进制、没有扩展名的：不是', () => {
    for (const n of ['shot.png', 'a.zip', 'app.exe', 'build.tar.gz', 'report.pdf', 'blob', '.txt.bak', 'txt'])
      expect(isTextFile(n), n).toBe(false);
    expect(isTextFile('blob', 'application/octet-stream')).toBe(false);
  });
});

describe('readHead', () => {
  it('小文件全部读完', async () => {
    const r = await readHead(new Response('第一行\n第二行\n'), 1024);
    expect(r).toEqual({ text: '第一行\n第二行\n', truncated: false, bytes: new TextEncoder().encode('第一行\n第二行\n').length });
  });

  it('大文件只读前 limit 个字节，截断处不完整的汉字丢掉', async () => {
    // 「中」是 3 个字节：limit 4 截在第二个字中间
    const r = await readHead(new Response('中文'), 4);
    expect(r).toEqual({ text: '中', truncated: true, bytes: 4 });
  });

  it('分好几块到达时照样按字节截断', async () => {
    const enc = new TextEncoder();
    const body = new ReadableStream<Uint8Array>({
      start(c) {
        for (let i = 0; i < 10; i++) c.enqueue(enc.encode(`line ${i}\n`));
        c.close();
      },
    });
    const r = await readHead(new Response(body), 14);
    expect(r).toEqual({ text: 'line 0\nline 1\n', truncated: true, bytes: 14 });
  });
});
