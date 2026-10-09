import { readFile } from "node:fs/promises";
import { compileFunction } from "node:vm";

// Run the actual shared browser script with an isolated window object. No DOM,
// application, model or native recording APIs are involved.
export async function loadWhiteboardLayout(before = false) {
  const file = before === "identity" ? "./fixtures/whiteboard-identity-before.js"
    : before === "intersection" ? "./fixtures/whiteboard-intersection-before.js"
    : before === "label-bound" ? "./fixtures/whiteboard-label-bound-before.js"
    : before ? "./fixtures/whiteboard-layout-before.js" : "../static/whiteboard-layout.js";
  const source = await readFile(new URL(file, import.meta.url), "utf8");
  // Use the host's built-ins for both versions, avoiding context-proxy costs
  // for each Math call while retaining separate script closures / exports.
  return compileFunction(source + "\nreturn window.WhiteboardLayout;", ["window"], { filename: file })({});
}
