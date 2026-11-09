export class BurstGate {
  private counts = new Map<string, number>();

  constructor(readonly limit = 6) {
    if (!Number.isInteger(limit) || limit < 1) throw new Error("positive integer limit required");
  }

  accept(key: string): boolean {
    const count = this.counts.get(key) ?? 0;
    if (count >= this.limit) return false;
    this.counts.set(key, count + 1);
    return true;
  }

  reset(key: string): void { this.counts.delete(key); }
}
