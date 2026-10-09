/** Native subscriptions may finish registering after their owning view closes. */
export type Cleanup = () => void | Promise<void>;
const noop = () => {};

export class ResourceScope {
  private alive = true;
  private cleanups = new Set<() => void>();

  constructor(private onCleanupError: (error: unknown) => void = (error) => {
    console.warn("[Selah] resource cleanup failed:", error);
  }) {}

  get active(): boolean { return this.alive; }

  fork(): ResourceScope {
    const child = new ResourceScope(this.onCleanupError);
    const release = this.own(() => child.dispose());
    // Closing a child also removes its ownership entry from the parent.
    child.own(release);
    return child;
  }

  guard<Args extends unknown[]>(callback: (...args: Args) => void): (...args: Args) => void {
    return (...args) => { if (this.alive) callback(...args); };
  }

  own(cleanup: Cleanup): () => void {
    let owned = true;
    const release = () => {
      if (!owned) return;
      owned = false;
      this.cleanups.delete(release);
      try {
        const result = cleanup();
        if (result) void Promise.resolve(result).catch(this.onCleanupError);
      } catch (error) {
        this.onCleanupError(error);
      }
    };
    if (this.alive) this.cleanups.add(release);
    else release();
    return release;
  }

  async acquire(register: () => Promise<Cleanup>): Promise<() => void> {
    if (!this.alive) return noop;
    return this.own(await register());
  }

  schedule(callback: () => void, delay: number): () => void {
    if (!this.alive) return noop;
    let pending = true;
    const timer = setTimeout(() => {
      if (!pending) return;
      release();
      if (this.alive) callback();
    }, delay);
    const release = this.own(() => {
      pending = false;
      clearTimeout(timer);
    });
    return release;
  }

  /** Releasing a repeating UI timer also invalidates already queued ticks. */
  interval(callback: () => void, delay: number): () => void {
    if (!this.alive) return noop;
    let running = true;
    const timer = setInterval(() => {
      if (this.alive && running) callback();
    }, delay);
    return this.own(() => {
      running = false;
      clearInterval(timer);
    });
  }

  dispose(): void {
    if (!this.alive) return;
    this.alive = false;
    // Disable callbacks before unsubscribing, including events already queued.
    for (const cleanup of [...this.cleanups].reverse()) cleanup();
  }
}

/** Only the most recently requested conversation subscription owns the slot. */
export class ResourceSlot {
  private version = 0;
  private release = noop;
  private registrationScope: ResourceScope | null = null;

  constructor(private scope: ResourceScope) {
    scope.own(() => this.clear());
  }

  clear(): void {
    this.version += 1;
    this.release();
    this.release = noop;
    this.registrationScope?.dispose();
    this.registrationScope = null;
  }

  async replace(register: (current: () => boolean, resources: ResourceScope) => Promise<Cleanup>): Promise<boolean> {
    this.clear();
    if (!this.scope.active) return false;
    const version = this.version;
    const registrationScope = this.scope.fork();
    this.registrationScope = registrationScope;
    const current = () => registrationScope.active && version === this.version;
    if (!current()) return false;
    let release: () => void;
    try {
      release = await registrationScope.acquire(() => register(current, registrationScope));
    } catch (error) {
      const obsolete = !current();
      registrationScope.dispose();
      if (obsolete) return false;
      throw error;
    }
    if (!current()) {
      release();
      registrationScope.dispose();
      return false;
    }
    this.release = release;
    return true;
  }
}

/** Register a set of subscriptions as one resource. A failed registration
 * disables the whole group immediately, including registrations still pending.
 * Factories must guard their callbacks with the provided child scope. */
export async function acquireResourceGroup(
  parent: ResourceScope,
  registrations: Array<(group: ResourceScope) => Promise<Cleanup>>,
): Promise<Cleanup> {
  const group = parent.fork();
  const release = () => group.dispose();
  try {
    await Promise.all(registrations.map((register) => group.acquire(() => register(group))));
    return release;
  } catch (error) {
    release();
    throw error;
  }
}
