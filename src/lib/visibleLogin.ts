import { ResourceScope, acquireResourceGroup, type Cleanup } from "./resourceScope";

export interface UniversityIdentity {
  username: string;
  display_name: string;
  student_id: string;
  faculty: string;
  department: string;
}

export interface UniversityLoginComplete {
  luna_authenticated: boolean;
  kwic_authenticated: boolean;
}

interface VisibleLoginDependencies {
  listen: <T>(name: string, receive: (event: { payload: T }) => void) => Promise<Cleanup>;
  open: () => Promise<void>;
  onIdentity: (identity: UniversityIdentity) => void;
  setProgress: (running: boolean) => void;
  onCleanupError?: (error: unknown) => void;
}

/** One attempt owns even handles which arrive after a terminal notification. */
export function waitForVisibleLogin(deps: VisibleLoginDependencies): Promise<UniversityLoginComplete> {
  return new Promise((resolve, reject) => {
    const resources = new ResourceScope(deps.onCleanupError);
    function finish(outcome: { value: UniversityLoginComplete } | { error: unknown }) {
      if (!resources.active) return;
      resources.dispose();
      try {
        deps.setProgress(false);
      } catch (error) {
        reject(error);
        return;
      }
      if ("error" in outcome) reject(outcome.error);
      else resolve(outcome.value);
    }

    async function initialize() {
      await acquireResourceGroup(resources, [
        group => deps.listen<UniversityIdentity>("login-success", group.guard(event => {
          try {
            deps.onIdentity(event.payload);
          } catch (error) {
            finish({ error });
          }
        })),
        group => deps.listen<UniversityLoginComplete>("university-login-complete", group.guard(event => {
          finish({ value: event.payload });
        })),
        group => deps.listen<string>("login-error", group.guard(() => {
          finish({ error: new Error("再ログインに失敗しました") });
        })),
        group => deps.listen<string>("login-cancelled", group.guard(() => {
          finish({ error: new Error("__login_cancelled__") });
        })),
      ]);
      if (!resources.active) return;
      await deps.open();
    }

    try {
      deps.setProgress(true);
      void initialize().catch(error => finish({ error }));
    } catch (error) {
      finish({ error });
    }
  });
}
