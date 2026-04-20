import { invoke } from "@tauri-apps/api/core";

export interface ExternalLinkDelegateOptions {
  scopeSelector?: string;
}

function findExternalAnchor(
  target: EventTarget | null,
  scopeSelector?: string
): HTMLAnchorElement | null {
  if (!(target instanceof Element)) return null;
  const anchor = target.closest("a");
  if (!(anchor instanceof HTMLAnchorElement)) return null;
  if (scopeSelector && !anchor.closest(scopeSelector)) return null;

  const href = anchor.getAttribute("href");
  if (!href) return null;
  if (!href.startsWith("http://") && !href.startsWith("https://")) return null;
  return anchor;
}

export function externalLinkDelegate(
  node: HTMLElement,
  options: ExternalLinkDelegateOptions = {}
) {
  let currentOptions = options;

  const handleClick = (event: MouseEvent) => {
    const anchor = findExternalAnchor(event.target, currentOptions.scopeSelector);
    if (!anchor) return;

    const href = anchor.getAttribute("href");
    if (!href) return;

    event.preventDefault();
    event.stopPropagation();
    invoke("open_external_url", { url: href }).catch((err) => {
      console.error("open_external_url failed:", err);
    });
  };

  node.addEventListener("click", handleClick);

  return {
    update(nextOptions: ExternalLinkDelegateOptions = {}) {
      currentOptions = nextOptions;
    },
    destroy() {
      node.removeEventListener("click", handleClick);
    },
  };
}
