import { describe, expect, it } from 'vitest';
import { blankForm, cmdLine, fromForm, templateError, templateRows, toForm } from './templates';
import type { AgentTemplate, HostInfo } from './types';

const tpl = (more: Partial<AgentTemplate> = {}): AgentTemplate => ({ id: 'kimi', name: 'Kimi', cmd: ['kimi', '-p', '{prompt}'], model: '', modelArgs: [], effortArgs: [], efforts: [], win: [], ...more });

describe('agent 模板的表单', () => {
  it('一行一个参数、强度用空格或逗号隔开、启动环境按固定顺序', () => {
    const t = fromForm({ ...blankForm(), id: ' kimi ', cmd: 'kimi\n -p \n\n{prompt}', modelArgs: '--model\n{model}', efforts: 'low, medium，high', win: ['cmd', 'pwsh'] });
    expect(t).toEqual({ id: 'kimi', name: '', cmd: ['kimi', '-p', '{prompt}'], model: '', modelArgs: ['--model', '{model}'], effortArgs: [], efforts: ['low', 'medium', 'high'], win: ['pwsh', 'cmd'] });
    expect(fromForm(toForm(t))).toEqual(t);
  });
  it('校验和服务端一致', () => {
    expect(templateError(tpl(), [], false)).toBeNull();
    expect(templateError(tpl({ id: 'Kimi' }), [], false)).toContain('标识');
    expect(templateError(tpl(), ['kimi'], false)).toContain('已经有');
    expect(templateError(tpl(), ['kimi'], true)).toBeNull();
    expect(templateError(tpl({ cmd: [] }), [], false)).toContain('不能为空');
    expect(templateError(tpl({ cmd: ['{prompt}'] }), [], false)).toContain('第一行');
    expect(templateError(tpl({ cmd: ['kimi'] }), [], false)).toContain('正好有一个');
    expect(templateError(tpl({ modelArgs: ['--model'] }), [], false)).toContain('{model}');
    expect(templateError(tpl({ effortArgs: ['--effort'] }), [], false)).toContain('{effort}');
    expect(templateError(tpl({ efforts: ['very high'] }), [], false)).toContain('思考强度');
  });
  it('命令的一行展示', () => {
    expect(cmdLine(['kimi', '-p', '{prompt}', '--name', 'a b'])).toBe('kimi -p {prompt} --name "a b"');
  });
});

describe('模板在各台机器上', () => {
  const host = (name: string, online: boolean, agents: HostInfo['agents']) => ({ name, online, agents }) as unknown as HostInfo;
  it('找到、没找到、本机配置覆盖、还没上报、离线', () => {
    const rows = templateRows(
      [
        host('a', true, [{ id: 'kimi', name: 'Kimi', version: '1.0', found: true }]),
        host('b', true, [{ id: 'kimi', name: 'Kimi', version: '', found: false }]),
        host('c', true, [{ id: 'kimi', name: 'Kimi', version: '2', found: true, local_override: true }]),
        host('d', true, []),
        host('e', false, [{ id: 'kimi', name: 'Kimi', version: '1.0', found: true }]),
      ],
      tpl(),
    );
    expect(rows.map(r => r.state)).toEqual(['found', 'missing', 'local', 'pending', 'offline']);
    expect(rows[0].version).toBe('1.0');
  });
});
