import { describe, expect, it } from 'vitest';
import { dirOf, projectDirError } from './projectDir';

describe('项目目录（#488）', () => {
  const others = [{ key: 'site', dir: '' }, { key: 'api', dir: 'backend' }];
  it('留空、一个合法的目录名都行；和自己的标识一样也行', () => {
    expect(projectDirError('', 'default')).toBeNull();
    expect(projectDirError('  board ', 'default', others)).toBeNull();
    expect(projectDirError('default', 'default', others)).toBeNull();
    expect(projectDirError('my.repo-2', 'default')).toBeNull();
  });
  it('路径、. ..、盘符、~、Windows 不允许的字符、以 . 结尾、太长都不行', () => {
    for (const bad of ['a/b', 'a\\b', '.', '..', 'C:', 'C:\\repo', '~repo', 'a|b', 'a?b', 'a*', 'a"b', 'repo.', 'x'.repeat(65), 'a\u0001b']) {
      expect(projectDirError(bad, 'default'), bad).not.toBeNull();
    }
  });
  it('不能和别的项目实际用的目录重名（不分大小写）：没填的按项目标识算', () => {
    expect(projectDirError('site', 'default', others)).toContain('site');
    expect(projectDirError('Backend', 'default', others)).toContain('api');
    expect(projectDirError('api', 'default', others)).toBeNull();
    // 留空（用项目标识）也查：别的项目把目录填成了这个标识
    expect(projectDirError('', 'backend', others)).toContain('api');
  });
  it('实际用的目录：填了用它，没填用项目标识', () => {
    expect(dirOf({ key: 'default', dir: 'board' })).toBe('board');
    expect(dirOf({ key: 'default', dir: '' })).toBe('default');
    expect(dirOf({ key: 'default' })).toBe('default');
  });
});
