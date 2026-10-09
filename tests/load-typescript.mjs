import { build } from "esbuild";

export async function loadTypeScript(path) {
  const result = await build({
    entryPoints: [path], bundle: true, write: false, platform: "node", format: "esm",
  });
  return import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString("base64")}`);
}
