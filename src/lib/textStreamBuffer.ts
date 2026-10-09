type Schedule = (callback: () => void, delay: number) => () => void;

/** Batch tokens for presentation without dropping or truncating their text. */
export class TextStreamBuffer {
  private chunks: string[] = [];
  private cancel: (() => void) | null = null;
  private active = true;

  constructor(
    private emit: (text: string) => void,
    private schedule: Schedule,
    private interval = 48,
  ) {}

  append(text: string): void {
    if (!this.active || !text) return;
    this.chunks.push(text);
    if (!this.cancel) this.cancel = this.schedule(() => this.flush(), this.interval);
  }

  flush(): void {
    const cancel = this.cancel;
    this.cancel = null;
    cancel?.();
    if (!this.active || this.chunks.length === 0) return;
    const text = this.chunks.join("");
    this.chunks = [];
    this.emit(text);
  }

  clear(): void {
    this.cancel?.();
    this.cancel = null;
    this.chunks = [];
  }

  dispose(): void {
    this.active = false;
    this.clear();
  }
}
