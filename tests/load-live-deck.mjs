import { build } from "esbuild";
import { compile } from "svelte/compiler";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";

/** Compile real card markup for SSR comparisons, sharing one Svelte runtime. */
export async function loadLiveDeck(name, previous = false) {
  const sources = resolve("src/lib/views/live");
  const legacy = resolve("tests/fixtures");
  const entry = previous ? resolve(legacy, `${name}-before.svelte`) : resolve(sources, `${name}.svelte`);
  const result = await build({
    stdin: {
      contents: `export { default as Component } from ${JSON.stringify(entry)}; export { render } from "svelte/server";`,
      resolveDir: process.cwd(), loader: "js",
    },
    bundle: true, write: false, platform: "node", format: "esm",
    plugins: [{
      name: "live-deck-server",
      setup(builder) {
        if (previous) {
          builder.onResolve({ filter: /^\.\/LiveSummaryCard\.svelte$/ }, () => ({
            path: resolve(legacy, "LiveSummaryCard-before.svelte"),
          }));
        }
        builder.onLoad({ filter: /\.svelte$/ }, async ({ path }) => {
          const source = (await readFile(path, "utf8")).split("<style>", 1)[0];
          const { js } = compile(source, { filename: path, generate: "server" });
          return { contents: js.code, loader: "js", resolveDir: sources };
        });
      },
    }],
  });
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString("base64")}`);
}
