/** A WebView's recovery work belongs to one native authentication generation. */
export class SessionLifetime {
  private epoch = 0;
  private generation = 0;
  private revision = -1;
  private pending = new Map<string, Promise<unknown>>();
  private tail: Promise<void> = Promise.resolve();

  ticket(): number { return this.epoch; }
  current(ticket: number): boolean { return ticket === this.epoch; }
  assertCurrent(ticket: number): void {
    if (!this.current(ticket)) throw new Error("University authentication operation was cancelled");
  }

  retire(): void {
    this.epoch += 1;
    this.pending.clear();
    this.tail = Promise.resolve();
  }

  accept(generation: number): boolean {
    if (generation < this.generation) return false;
    if (generation > this.generation) {
      this.generation = generation;
      this.revision = -1;
      this.retire();
    }
    return true;
  }

  acceptSnapshot(generation: number, revision: number): boolean {
    if (!this.accept(generation) || revision < this.revision) return false;
    this.revision = revision;
    return true;
  }

  recover<T>(service: string, run: () => Promise<T>): Promise<T> {
    const existing = this.pending.get(service);
    if (existing) return existing as Promise<T>;
    const ticket = this.ticket();
    const request = this.tail.catch(() => {}).then(async () => {
      this.assertCurrent(ticket);
      const result = await run();
      this.assertCurrent(ticket);
      return result;
    }).finally(() => {
      if (this.pending.get(service) === request) this.pending.delete(service);
    });
    this.tail = request.then(() => {}, () => {});
    this.pending.set(service, request);
    return request;
  }
}

export const universitySessionLifetime = new SessionLifetime();
