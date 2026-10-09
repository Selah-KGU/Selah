import { LatestViewRead } from "./latestViewRead";
import type { ResourceScope } from "./resourceScope";
import { getSttStreamState, startSttStream, stopSttStream,
  type SttStreamOwner, type SttStreamState } from "./sttSessionApi";

export interface AgentSpeechEvent {
  caller: string;
  input_session_id?: string;
}

interface InputSession {
  owner: SttStreamOwner;
  pending: boolean;
  ended: boolean;
  previous: SttStreamOwner | null;
  restored: boolean;
  released: boolean;
}

interface SpeechTransport {
  start: typeof startSttStream;
  stop: typeof stopSttStream;
  read: typeof getSttStreamState;
  createId: () => string;
}

/** One view owns only the input UUID it requested, including while start is
 * queued. Cleanup never stops a replacement Agent input with the same caller. */
export class AgentSpeechInput {
  private session: InputSession | null = null;
  private read: LatestViewRead<{ session: InputSession; state: SttStreamState }>;

  constructor(
    private scope: ResourceScope,
    private changed: (active: boolean) => void,
    private transport: SpeechTransport = {
      start: startSttStream, stop: stopSttStream, read: getSttStreamState,
      createId: () => crypto.randomUUID(),
    },
  ) {
    this.read = new LatestViewRead(scope, async () => {
      const session = this.session!;
      return { session, state: await this.transport.read() };
    }, ({ session, state }) => {
      if (this.session !== session) return;
      const active = state.owner?.caller === "agent" &&
        state.owner.input_session_id === session.owner.input_session_id &&
        (state.phase === "initializing" || state.phase === "listening");
      this.changed(active);
      if (!active) {
        session.ended = true;
        void this.restore(session);
      }
    });
    scope.own(() => {
      const session = this.session;
      this.session = null;
      this.read.invalidate();
      if (session) void this.release(session).catch(error => this.cleanupError(error));
    });
  }

  accepts(event: AgentSpeechEvent): boolean {
    return this.scope.active && !!this.session && event.caller === "agent" &&
      event.input_session_id === this.session.owner.input_session_id;
  }

  state(event: AgentSpeechEvent & { state: string }): void {
    if (!this.accepts(event)) return;
    this.read.invalidate();
    const active = event.state === "initializing" || event.state === "listening";
    this.changed(active);
    if (!active) {
      const session = this.session!;
      session.ended = true;
      void this.restore(session);
    }
  }

  error(event: AgentSpeechEvent): void {
    if (!this.accepts(event)) return;
    this.read.invalidate();
    this.changed(false);
    // Idle follows native teardown; restoring before it would race final decode.
  }

  async start(): Promise<boolean> {
    if (!this.scope.active || (this.session && (this.session.pending || !this.session.ended))) return false;
    const session: InputSession = {
      owner: { caller: "agent", live_session_id: null, input_session_id: this.transport.createId() },
      pending: true, ended: false, previous: null, restored: false, released: false,
    };
    this.session = session;
    this.read.invalidate();
    try {
      session.previous = await this.transport.start(session.owner, true);
      session.pending = false;
      if (!this.scope.active || this.session !== session) {
        await this.release(session);
        return false;
      }
      // An idle event can beat the start response (e.g. initialization failed).
      if (session.ended) await this.restore(session);
      else await this.refresh();
      return true;
    } catch (error) {
      session.pending = false;
      if (!this.scope.active || this.session !== session) {
        this.cleanupError(error);
        return false;
      }
      this.session = null;
      this.read.invalidate();
      this.changed(false);
      throw error;
    }
  }

  async stop(): Promise<void> {
    if (!this.scope.active || !this.session) return;
    this.read.invalidate();
    await this.transport.stop(this.session.owner);
  }

  async refresh(): Promise<boolean> {
    if (!this.scope.active || !this.session) return false;
    try { return await this.read.refresh(); }
    catch (error) {
      if (this.scope.active) console.warn("[Agent] STT state read failed:", error);
      return false;
    }
  }

  private async release(session: InputSession): Promise<void> {
    if (session.released) return;
    // The first scoped stop can run before a queued start reserves the mic.
    // Once that start returns, stop again using the same UUID, then restore.
    if (!session.pending) session.released = true;
    await this.transport.stop(session.owner);
    if (!session.pending) {
      session.ended = true;
      await this.restore(session);
    }
  }

  private async restore(session: InputSession): Promise<void> {
    if (session.pending || !session.previous || session.restored) return;
    session.restored = true;
    // A replacement input or an ended LIVE rejects this non-preempting start.
    try { await this.transport.start(session.previous); }
    catch { /* The previous owner may have ended or a new input may own the mic. */ }
  }

  private cleanupError(error: unknown): void {
    console.warn("[Agent] owned voice input cleanup failed:", error);
  }
}
