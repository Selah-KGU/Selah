// Actual Markdown consumers and loader, with two deliberately failing asset
// requests followed by recovery. No app/native APIs. Open the printed URL.
import { build } from "esbuild";
import { compile } from "svelte/compiler";
import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { createServer } from "node:http";

const source = `
import { mount, unmount } from "svelte";
import Harness from ${JSON.stringify(resolve("tests/fixtures/whiteboard-loader-browser.svelte"))};
let app;
window.__loaderProbe = {
  async finish(result) {
    await unmount(app);
    result.checks.push("consumers unmounted after recovery");
    const output = document.createElement("pre"); output.textContent = JSON.stringify(result, null, 2); document.body.append(output);
    await fetch("/result", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(result) });
  }
};
app = mount(Harness, { target: document.getElementById("probe") });
`;
const bundle = await build({
  stdin: { contents: source, resolveDir: process.cwd(), loader: "js" },
  bundle: true, write: false, platform: "browser", format: "esm", conditions: ["browser"],
  plugins: [{ name: "loader-consumers", setup(builder) {
    builder.onLoad({ filter: /\.svelte$/ }, async ({ path }) => ({
      contents: compile(await readFile(path, "utf8"), { filename: path, generate: "client", css: "injected" }).js.code,
      loader: "js", resolveDir: dirname(path),
    }));
  } }],
});
const layout = await readFile("static/whiteboard-layout.js", "utf8");
let attempts = 0;
const server = createServer(async (request, response) => {
  response.setHeader("Cache-Control", "no-store");
  if (request.url === "/probe.js") {
    response.setHeader("Content-Type", "text/javascript"); response.end(bundle.outputFiles[0].text);
  } else if (request.url === "/whiteboard-layout.js") {
    attempts++;
    if (attempts <= 2) { response.statusCode = 404; response.end("intentional test failure"); }
    else { response.setHeader("Content-Type", "text/javascript"); response.end(layout); }
  } else if (request.url === "/status") {
    response.setHeader("Content-Type", "application/json"); response.end(JSON.stringify({ attempts }));
  } else if (request.url === "/result" && request.method === "POST") {
    const chunks = []; for await (const chunk of request) chunks.push(chunk);
    const result = JSON.parse(Buffer.concat(chunks).toString());
    console.log(JSON.stringify({ passed: result.checks.length, attempts, ...result })); response.end("ok");
  } else if (request.url === "/") {
    response.setHeader("Content-Type", "text/html");
    response.end('<!doctype html><meta charset="utf-8"><title>Whiteboard loader verification</title><div id="probe"></div><script type="module" src="/probe.js"></script>');
  } else { response.statusCode = 404; response.end(); }
});
server.listen(0, "127.0.0.1", () => console.log(`http://127.0.0.1:${server.address().port}/`));
