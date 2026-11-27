import { describe, expect, it } from 'vitest';
import { replaceMark } from './attach';

describe('附件上传占位替换', () => {
  it('文件名含 $\' 时只换掉占位，其余文字原样保留', () => {
    const mark = "![上传中：a$'b.txt…](#upload-1)";
    const md = "[a$'b.txt](/api/files/1)";
    expect(replaceMark(`前文\n${mark}\n后文`, mark, md)).toBe(`前文\n${md}\n后文`);
  });

  it('$& 和 $$ 也按原样写进链接', () => {
    const mark = '![上传中：a$&b$$.txt…](#upload-2)';
    const md = '[a$&b$$.txt](/api/files/2)';
    expect(replaceMark(mark, mark, md)).toBe(md);
  });

  it('失败时去掉占位和它旁边的一个换行', () => {
    const mark = '![上传中：a.txt…](#upload-3)';
    expect(replaceMark(`前文\n${mark}\n后文`, mark, '')).toBe('前文\n后文');
  });
});
