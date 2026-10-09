import { readFile } from 'node:fs/promises';

/** Decode the exact Rust format string passed to WebView.eval. */
export async function loadFrontendHealthProbe(path = process.env.SELAH_HEALTH_PROBE_BEFORE || 'src-tauri/src/frontend_health.rs', sequence = 7) {
  const source = await readFile(path, 'utf8');
  const script = source.match(/r#"(\(\(\) => [\s\S]*?)"#/)?.[1];
  if (!script) throw new Error(`Native frontend-health probe missing: ${path}`);
  return script.replaceAll('{sequence}', String(sequence)).replaceAll('{{', '{').replaceAll('}}', '}');
}
