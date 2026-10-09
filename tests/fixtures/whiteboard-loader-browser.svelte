<script lang="ts">
  import { onMount, tick } from "svelte";
  import MarkdownWhiteboard from "../../src/lib/MarkdownWhiteboard.svelte";
  import { prepareWhiteboardLayout, whiteboardLayoutReady } from "../../src/lib/whiteboardLayout";
  import { boardFixture } from "./whiteboard-layout-cases.mjs";

  let boards = $state.raw<any[]>([]);
  let unrelated = $state(0);
  const fresh = (title: string) => ({ ...boardFixture({ count: 12, edges: 18 }), title });
  const attempts = async () => (await (await fetch("/status")).json()).attempts;
  const scripts = () => document.querySelectorAll('script[data-whiteboard-layout="1"]').length;
  const deadline = async (promise: Promise<void>) => {
    let timer: ReturnType<typeof setTimeout>;
    try {
      await Promise.race([promise, new Promise<void>((_, reject) => { timer = setTimeout(() => reject(new Error("loader did not settle")), 3000); })]);
    } finally { clearTimeout(timer!); }
  };
  onMount(() => { void run(); });
  async function run() {
    const checks: string[] = [];
    const check = (condition: unknown, label: string) => { if (!condition) throw new Error(label); checks.push(label); };
    try {
      await tick();
      check(scripts() === 0 && await attempts() === 0, "importing both actual modules does not request a script");
      check(!$whiteboardLayoutReady, "readiness is false before the first board");
      boards = [fresh("initial A"), fresh("initial B")]; await tick();
      check(scripts() === 1, "two mounted consumers share one pending script");
      let error: unknown;
      try { await deadline(prepareWhiteboardLayout()); } catch (failure) { error = failure; }
      await tick();
      check(error instanceof Error && error.message.includes("failed to load"), "persistent asset errors settle rather than hanging");
      check(await attempts() === 2, "concurrent consumers share one retry and stop after two errors");
      check(scripts() === 0 && !$whiteboardLayoutReady, "failed attempts remove their scripts and do not mark ready");
      check(document.querySelectorAll(".whiteboard").length === 0, "failed loading does not render a broken whiteboard");
      boards = [fresh("recovered A"), fresh("recovered B")]; await tick();
      await deadline(prepareWhiteboardLayout()); await tick();
      check(await attempts() === 3 && scripts() === 1, "a new board recovers with one fresh successful script");
      check($whiteboardLayoutReady, "successful loading publishes readiness");
      check(document.querySelectorAll(".whiteboard").length === 2, "readiness renders both actual Markdown consumers");
      check(document.querySelectorAll(".wb-node").length > 0, "late loaded layout renders actual nodes");
      check(document.querySelector('.whiteboard[aria-label="recovered A"]') && document.querySelector('.whiteboard[aria-label="recovered B"]'), "both complete board titles survive recovery");
      for (let i = 0; i < 100; i++) { unrelated++; await tick(); }
      check(await attempts() === 3 && scripts() === 1, "100 unrelated updates do not reload the engine");
      boards = []; await tick();
      check(document.querySelectorAll(".whiteboard").length === 0, "closing consumers removes their pages");
      boards = [fresh("reopened")]; await tick();
      check(document.querySelectorAll(".whiteboard").length === 1 && await attempts() === 3, "reopening reuses the loaded engine without another request");
      await (window as any).__loaderProbe.finish({ checks, error: null });
    } catch (error) { await (window as any).__loaderProbe.finish({ checks, error: String(error) }); }
  }
</script>

<p>Isolated loader recovery — unrelated update {unrelated}</p>
{#each boards as board, index (index)}
  <MarkdownWhiteboard {board} />
{/each}
