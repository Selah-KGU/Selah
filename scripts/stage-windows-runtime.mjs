import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const DLL_NAMES = new Set([
  "sherpa-onnx-c-api.dll",
  "onnxruntime.dll",
  "onnxruntime_providers_shared.dll",
  "sherpa-onnx-cxx-api.dll",
  "directml.dll",
]);

function log(message) {
  console.log(`[stage-windows-runtime] ${message}`);
}

async function exists(target) {
  try {
    await fs.access(target);
    return true;
  } catch {
    return false;
  }
}

async function walk(dir, matches) {
  let entries;
  try {
    entries = await fs.readdir(dir, { withFileTypes: true });
  } catch {
    return;
  }

  await Promise.all(entries.map(async (entry) => {
    const fullPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name === "bundle" || entry.name === "windows-runtime") {
        return;
      }
      await walk(fullPath, matches);
      return;
    }
    if (entry.isFile() && DLL_NAMES.has(entry.name.toLowerCase())) {
      matches.push(fullPath);
    }
  }));
}

async function newest(paths) {
  const withStats = await Promise.all(paths.map(async (file) => ({
    file,
    stat: await fs.stat(file),
  })));
  withStats.sort((a, b) => b.stat.mtimeMs - a.stat.mtimeMs);
  return withStats[0]?.file ?? null;
}

async function main() {
  if (process.platform !== "win32") {
    log("non-Windows host; skipping");
    return;
  }

  const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  const tauriDir = path.join(repoRoot, "src-tauri");
  const stageDir = path.join(tauriDir, "windows-runtime");

  const targetDirEnv = process.env.CARGO_TARGET_DIR;
  const targetRoot = targetDirEnv
    ? path.isAbsolute(targetDirEnv)
      ? targetDirEnv
      : path.resolve(tauriDir, targetDirEnv)
    : path.join(tauriDir, "target");

  if (!(await exists(targetRoot))) {
    throw new Error(`target dir not found: ${targetRoot}`);
  }

  const matches = [];
  await walk(targetRoot, matches);

  const sherpaDll = await newest(
    matches.filter((file) => path.basename(file).toLowerCase() === "sherpa-onnx-c-api.dll"),
  );

  if (!sherpaDll) {
    throw new Error(`sherpa-onnx-c-api.dll not found under ${targetRoot}`);
  }

  const runtimeDir = path.dirname(sherpaDll);
  const directMlDll = await newest(
    matches.filter((file) => path.basename(file).toLowerCase() === "directml.dll"),
  );

  await fs.rm(stageDir, { recursive: true, force: true });
  await fs.mkdir(stageDir, { recursive: true });

  const runtimeEntries = await fs.readdir(runtimeDir, { withFileTypes: true });
  for (const entry of runtimeEntries) {
    if (!entry.isFile() || path.extname(entry.name).toLowerCase() !== ".dll") {
      continue;
    }
    const src = path.join(runtimeDir, entry.name);
    const dst = path.join(stageDir, entry.name);
    await fs.copyFile(src, dst);
  }

  if (directMlDll) {
    const dst = path.join(stageDir, "DirectML.dll");
    if (!(await exists(dst))) {
      await fs.copyFile(directMlDll, dst);
    }
  }

  const staged = await fs.readdir(stageDir);
  log(`staged ${staged.length} DLL(s) from ${runtimeDir}`);
}

main().catch((error) => {
  console.error(`[stage-windows-runtime] ${error.message}`);
  process.exit(1);
});
