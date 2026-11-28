/* ==========================================================================
   小 LRU：界面只留最近用过的少量对象，超出后丢掉最久没碰的
   ========================================================================== */
export class Lru<K, V> {
  private map = new Map<K, V>();

  constructor(private max: number) {}

  get size() {
    return this.map.size;
  }

  get(key: K): V | undefined {
    const value = this.map.get(key);
    if (value === undefined) return undefined;
    this.map.delete(key);
    this.map.set(key, value);
    return value;
  }

  set(key: K, value: V): this {
    this.map.delete(key);
    this.map.set(key, value);
    while (this.map.size > this.max) {
      const oldest = this.map.keys().next();
      if (oldest.done) break;
      this.map.delete(oldest.value);
    }
    return this;
  }
}
