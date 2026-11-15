/* 服务端升级之后还开着的旧页面（default/main#368）：打包版本和服务端不一样时提示一次，同一份数据不重复提示 */
import { describe, expect, it } from 'vitest';
import { markStaleNoticed, sameVersion, staleNoticed, staleServer, staleText } from './stale';

describe('sameVersion', () => {
  it('三个数字一样就是同一版；预发布后缀不看', () => {
    expect(sameVersion('0.1.30', '0.1.30')).toBe(true);
    expect(sameVersion('0.1.30', 'v0.1.30')).toBe(true);
    expect(sameVersion('0.1.30', '0.1.30-build.5')).toBe(true);
  });

  it('差一位就是不同版本：按数字比，不按字符串', () => {
    expect(sameVersion('0.1.30', '0.1.31')).toBe(false);
    expect(sameVersion('0.1.9', '0.1.30')).toBe(false);
  });

  it('解析不出来的按原文比', () => {
    expect(sameVersion('dev', 'dev')).toBe(true);
    expect(sameVersion('dev', '0.1.30')).toBe(false);
  });
});

describe('staleServer', () => {
  it('版本不一样：返回服务端版本（提示里要用）', () => {
    expect(staleServer('0.1.30', '0.1.31')).toBe('0.1.31');
    expect(staleServer('0.1.31', '0.1.30')).toBe('0.1.30');
  });

  it('版本一样、不知道打包版本、服务端没给版本：都不提示', () => {
    expect(staleServer('0.1.31', '0.1.31')).toBeNull();
    expect(staleServer('', '0.1.31')).toBeNull();
    expect(staleServer('  ', '0.1.31')).toBeNull();
    expect(staleServer('0.1.30', '')).toBeNull();
    expect(staleServer('0.1.30', '  ')).toBeNull();
    expect(staleServer('0.1.30', null)).toBeNull();
    expect(staleServer('0.1.30', undefined)).toBeNull();
  });
});

describe('staleText', () => {
  it('带上服务端版本，不重复写 v', () => {
    expect(staleText('0.1.31')).toBe('服务端已经升级到 v0.1.31，刷新页面用新版');
    expect(staleText('v0.1.31')).toBe('服务端已经升级到 v0.1.31，刷新页面用新版');
  });
});

describe('staleNoticed', () => {
  it('同一个版本只提示一次，服务端再升一版时再提示', () => {
    expect(staleNoticed('0.1.31')).toBe(false);
    markStaleNoticed('0.1.31');
    expect(staleNoticed('0.1.31')).toBe(true);
    expect(staleNoticed('0.1.32')).toBe(false);
  });
});
