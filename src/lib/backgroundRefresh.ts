import { ResourceScope, type Cleanup } from "./resourceScope";

interface BackgroundRefreshDependencies {
  demo: () => boolean;
  visible: () => boolean;
  subscribe: (callback: () => void) => Cleanup;
  hydrate: () => void;
  catchUp: () => void;
  invalidate: () => void;
}

/** Native code owns periodic refresh. This owns the WebView's catch-up only. */
export class BackgroundRefresh {
  private resources: ResourceScope | null = null;

  constructor(private dependencies: BackgroundRefreshDependencies) {}

  start(refresh = false): void {
    if (this.dependencies.demo()) {
      this.stop();
      return;
    }
    if (this.resources?.active) {
      if (refresh) {
        this.dependencies.invalidate();
        this.dependencies.hydrate();
      }
      return;
    }
    const resources = new ResourceScope();
    this.resources = resources;
    resources.own(this.dependencies.subscribe(resources.guard(() => {
      if (!this.dependencies.demo() && this.dependencies.visible()) this.dependencies.catchUp();
    })));
    if (resources.active) this.dependencies.hydrate();
  }

  stop(): void {
    const resources = this.resources;
    this.resources = null;
    resources?.dispose();
    this.dependencies.invalidate();
  }
}
