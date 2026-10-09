/** Preserve the existing saved-note preview's section boundaries. */
export function extractOverallSummary(md: string): string {
  const start = md.indexOf("### 全体要約");
  if (start < 0) return "";
  const afterHeader = md.indexOf("\n", start);
  if (afterHeader < 0) return "";
  const nextSection = md.indexOf("\n###", afterHeader + 1);
  const end = nextSection >= 0 ? nextSection : md.indexOf("\n## ", afterHeader + 1);
  return (end >= 0 ? md.slice(afterHeader + 1, end) : md.slice(afterHeader + 1)).trim();
}
