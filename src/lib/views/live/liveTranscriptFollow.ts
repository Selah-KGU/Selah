import type { ResourceScope } from "../../resourceScope";

export type TranscriptFollowState = {
  recordingId: string | null;
  lineCount: number;
  target: Pick<HTMLElement, "scrollHeight" | "scrollTop"> | null;
  visible: boolean;
  listening: boolean;
  autoFollow: boolean;
};

type ScheduleFrame = (callback: () => void) => () => void;
type Frame = { cancel: () => void };
const scheduleFrame: ScheduleFrame = callback => {
  const id = requestAnimationFrame(callback);
  return () => cancelAnimationFrame(id);
};

/** One current frame follows the latest visible transcript, never an old pane. */
export class LiveTranscriptFollow {
  private state: TranscriptFollowState | null = null;
  private pending: Frame | null = null;
  private followedCount: number | null = null;
  private disposed = false;

  constructor(scope: ResourceScope, private schedule: ScheduleFrame = scheduleFrame) {
    scope.own(() => this.dispose());
  }

  update(state: TranscriptFollowState): void {
    if (this.disposed) return;
    if (this.state?.recordingId !== state.recordingId || this.state?.target !== state.target) {
      this.cancelPending();
      this.followedCount = null;
    }
    this.state = state;
    if (!state.recordingId || !state.target || !state.visible || !state.listening || !state.autoFollow) {
      this.cancelPending();
      this.followedCount = null;
      return;
    }
    if (this.pending || this.followedCount === state.lineCount) return;
    const frame: Frame = { cancel: () => {} };
    this.pending = frame;
    try {
      frame.cancel = this.schedule(() => {
        if (this.disposed || this.pending !== frame) return;
        this.pending = null;
        const latest = this.state!;
        latest.target!.scrollTop = latest.target!.scrollHeight;
        this.followedCount = latest.lineCount;
      });
    } catch (error) {
      if (this.pending === frame) this.pending = null;
      throw error;
    }
  }

  private cancelPending(): void {
    const frame = this.pending;
    this.pending = null;
    frame?.cancel();
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.state = null;
    this.cancelPending();
  }
}
