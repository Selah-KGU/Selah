/** Share a pending status read. A pushed update or lifetime change retires its
 * result and failure, while allowing the next owner to read immediately. */
export class CoalescedStatusRead<Value> {
  private version = 0;
  private pending: Promise<void> | null = null;

  constructor(
    private read: () => Promise<Value>,
    private apply: (value: Value) => void,
  ) {}

  invalidate(): void {
    this.version += 1;
    this.pending = null;
  }

  refresh(): Promise<void> {
    if (this.pending) return this.pending;
    const version = this.version;
    const current = () => version === this.version;
    // Establish ownership before read/apply can re-enter through store callbacks.
    const request = Promise.resolve().then(async () => {
      if (!current()) return;
      try {
        const value = await this.read();
        if (current()) this.apply(value);
      } catch (error) {
        if (current()) throw error;
      }
    }).finally(() => {
      if (this.pending === request) this.pending = null;
    });
    this.pending = request;
    return request;
  }
}
