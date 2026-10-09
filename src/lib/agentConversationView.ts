import { ResourceScope, ResourceSlot, type Cleanup } from "./resourceScope";

interface ConversationViewOptions<Messages, Event> {
  scope: ResourceScope;
  loadMessages: (id: string) => Promise<Messages>;
  listen: (id: string, receive: (event: Event) => void) => Promise<Cleanup>;
  select: (id: string | null, share: boolean) => void;
  applyMessages: (messages: Messages) => void;
  receive: (event: Event) => void;
}

/** Owns the displayed conversation, including A → B → A and late registrations.
 * Sending waits for both history and the subscription; recovery reads have their
 * own generation so they cannot erase a newly started answer in the same chat. */
export class AgentConversationView<Messages, Event> {
  private id: string | null = null;
  private version = 0;
  private messageVersion = 0;
  private ready = false;
  private pending: Promise<boolean> | null = null;
  private stream: ResourceSlot;

  constructor(private options: ConversationViewOptions<Messages, Event>) {
    this.stream = new ResourceSlot(options.scope);
  }

  capture(): () => boolean {
    const version = this.version;
    return () => this.options.scope.active && version === this.version;
  }

  invalidateMessages(): void {
    this.messageVersion += 1;
  }

  clear(share = true): void {
    this.version += 1;
    this.invalidateMessages();
    this.stream.clear();
    this.id = null;
    this.pending = null;
    this.ready = false;
    if (this.options.scope.active) this.options.select(null, share);
  }

  deleted(id: string): boolean {
    if (this.id !== id) return false;
    // The backend has already cleared only this selection in its transaction.
    // Publishing a generic empty selection here could overwrite a newer chat.
    this.clear(false);
    return true;
  }

  select(id: string, share = true): Promise<boolean> {
    if (!this.options.scope.active) return Promise.resolve(false);
    if (this.id === id) {
      if (this.pending) return this.pending;
      if (this.ready) return Promise.resolve(true);
    }
    this.version += 1;
    this.invalidateMessages();
    this.stream.clear();
    this.id = id;
    this.ready = false;
    const current = this.capture();
    this.options.select(id, share);
    const pending = Promise.all([
      this.reload(),
      this.stream.replace((registered) => this.options.listen(id, (event) => {
        if (current() && registered()) this.options.receive(event);
      })),
    ]).then(([loaded, listening]) => {
      if (!current()) return false;
      this.ready = loaded && listening;
      return this.ready;
    }).catch((error) => {
      if (!current()) return false;
      this.stream.clear();
      throw error;
    });
    this.pending = pending;
    // A failed selection can be retried; an older completion cannot clear the
    // next selection's pending promise or declare it ready.
    const finished = () => { if (this.pending === pending) this.pending = null; };
    void pending.then(finished, finished);
    return pending;
  }

  async reload(accept: () => boolean = () => true): Promise<boolean> {
    const id = this.id;
    const current = this.capture();
    const messageVersion = ++this.messageVersion;
    const applicable = () => current() && messageVersion === this.messageVersion && accept();
    if (!id || !applicable()) return false;
    try {
      const messages = await this.options.loadMessages(id);
      if (!applicable()) return false;
      this.options.applyMessages(messages);
      return true;
    } catch (error) {
      if (!applicable()) return false;
      throw error;
    }
  }
}
