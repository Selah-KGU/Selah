import type { ResourceScope } from "./resourceScope";

/** Read-only UI work: a new read, a pushed update, or disposal supersedes IO
 * already in flight. Only current failures reach callers or error presentation. */
export class LatestViewRead<Value, Args extends unknown[] = []> {
  private version = 0;

  constructor(
    private scope: ResourceScope,
    private read: (...args: Args) => Promise<Value>,
    private apply: (value: Value, ...args: Args) => void,
    private failed?: (error: unknown) => void,
  ) {}

  invalidate(): void { this.version += 1; }

  async refresh(...args: Args): Promise<boolean> {
    const version = ++this.version;
    const current = () => this.scope.active && version === this.version;
    if (!current()) return false;
    try {
      const value = await this.read(...args);
      if (!current()) return false;
      this.apply(value, ...args);
      return true;
    } catch (error) {
      if (!current()) return false;
      this.failed?.(error);
      throw error;
    }
  }
}
