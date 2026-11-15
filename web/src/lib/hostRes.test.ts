import { describe, expect, it } from 'vitest';
import { bytes, fullDisks, resLine, sparkPath, tiles } from './hostRes';
import type { HostInfo, HostRes } from './types';

const GB = 1024 ** 3;
const res = (x: Partial<HostRes> = {}): HostRes => ({
  at: 50_000,
  cpu: { pct: 34, cores: 16, model: 'AMD Ryzen 7 7840HS' },
  mem: { total: 32 * GB, used: 21.4 * GB },
  gpus: [{ name: 'NVIDIA GeForce RTX 4070', vendor: 'NVIDIA', pct: 58, vramUsed: 7.9 * GB, vramTotal: 12 * GB }],
  disks: [
    { mount: 'D:\\', total: 931 * GB, avail: 519 * GB, ws: true },
    { mount: 'C:\\', total: 476 * GB, avail: 33.6 * GB },
  ],
  hist: { cpu: [30, 34], mem: [66, 67], gpu: [50, 58] },
  ...x,
});
const host = (x: Partial<HostInfo> = {}): HostInfo => ({ name: 'win2', online: true, version: '0.1.31', res: res(), ...x }) as HostInfo;
const since = (t: number, now: number) => `${Math.round((now - t) / 1000)} 秒前`;

describe('机器资源', () => {
  it('字节写成 GB / TB', () => {
    expect(bytes(21.4 * GB)).toBe('21.4 GB');
    expect(bytes(32 * GB)).toBe('32 GB');
    expect(bytes(931 * GB)).toBe('931 GB');
    expect(bytes(1863 * GB)).toBe('1.8 TB');
  });

  it('四块磁贴：数值、细节、趋势；快满的盘警告色，工作空间的盘是大数字', () => {
    const [cpu, mem, gpu, disk] = tiles(res());
    expect(cpu).toMatchObject({ pct: 34, sub: '16 核 · AMD Ryzen 7 7840HS', hist: [30, 34] });
    expect(mem).toMatchObject({ pct: 67, sub: '21.4 GB / 32 GB', warn: false });
    expect(gpu).toMatchObject({ title: 'GPU', pct: 58, sub: 'NVIDIA GeForce RTX 4070 · 显存 7.9 GB / 12 GB' });
    expect(disk).toMatchObject({ pct: 44, sub: '剩 519 GB，共 931 GB', warn: false, disk: { mount: 'D:\\', ws: true } });
    expect(disk.more).toEqual([{ key: 'disk1', label: 'C:\\', pct: 93, sub: '剩 33.6 GB', warn: true }]);
  });

  it('负载写三个数，Apple 芯片的 GPU 和内存共用，几块 GPU 第一块是大数字', () => {
    const [cpu, , gpu] = tiles(
      res({
        cpu: { pct: 18, cores: 12, load: [2.14, 1.82, 22.6] },
        gpus: [
          { name: 'Apple M3 Pro', vendor: 'Apple Inc.', pct: 12 },
          { name: 'NVIDIA A10', pct: 0 },
        ],
      }),
    );
    expect(cpu.sub).toBe('12 核 · 负载 2.1 / 1.8 / 23');
    expect(gpu).toMatchObject({ title: 'GPU · 2 块', sub: 'Apple M3 Pro · 和内存共用' });
    expect(gpu.more).toEqual([{ key: 'gpu1', label: 'GPU 2', pct: 0, sub: 'NVIDIA A10', warn: false }]);
  });

  it('取不到的写「不支持」「未上报」「没有」，不画 0', () => {
    const [cpu, mem, gpu, disk] = tiles(res({ cpu: undefined, mem: undefined, gpus: undefined, disks: undefined }));
    expect([cpu.pct, mem.pct, gpu.pct, disk.pct]).toEqual([null, null, null, null]);
    expect([cpu.none, mem.none, gpu.none, disk.none]).toEqual(['未上报', '未上报', '不支持', '未上报']);
    expect(tiles(res({ gpus: [] }))[2]).toMatchObject({ pct: null, none: '没有' });
    // 读不到使用率的 GPU：不画趋势
    expect(tiles(res({ gpus: [{ name: 'Intel UHD' }] }))[2]).toMatchObject({ pct: null, none: '未上报', hist: [] });
  });

  it('内存、盘到 90% 起警告；快满的盘算进「设置 · N」', () => {
    const r = res({ mem: { total: 8 * GB, used: 7.5 * GB }, disks: [{ mount: '/', label: 'Macintosh HD', total: 78 * GB, avail: 3.1 * GB, ws: true }] });
    const [, mem, , disk] = tiles(r);
    expect(mem.warn).toBe(true);
    expect(disk).toMatchObject({ warn: true, disk: { mount: 'Macintosh HD', ws: true } });
    expect(fullDisks(host({ res: r }))).toBe(1);
    expect(fullDisks(host())).toBe(1);
    expect(fullDisks(host({ res: undefined }))).toBe(0);
  });

  it('标题旁那一句：在线、离线、旧版、还没采到', () => {
    expect(resLine(host(), 58_000, '0.1.31', false, since)).toBe('8 秒前采样 · 每 10 秒更新');
    expect(resLine(host({ online: false }), 58_000, '0.1.31', false, since)).toBe('离线前最后一次 · 8 秒前');
    expect(resLine(host({ res: undefined, version: '0.1.30' }), 0, '0.1.31', true, since)).toBe('board 0.1.30 不上报资源：升级到 0.1.31 后显示');
    expect(resLine(host({ res: undefined }), 0, '0.1.31', false, since)).toBe('还没采到资源：下一次上报带上');
    expect(resLine(host({ res: undefined, online: false }), 0, '0.1.31', false, since)).toBe('机器离线，还没上报过资源');
  });

  it('趋势线：少于两个点不画，数值夹在 0–100', () => {
    expect(sparkPath([5])).toBeNull();
    const p = sparkPath([0, 150])!;
    expect(p.line).toBe('M0.0,26.0L100.0,2.0');
    expect(p.area.endsWith('L100,28L0,28Z')).toBe(true);
  });
});
