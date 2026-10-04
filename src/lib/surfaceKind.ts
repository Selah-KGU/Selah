/** Which shell this WebView is. Kept dependency-free so every window can branch before importing the app. */
export function readSurfaceParam(name = "surface"): string {
  if (typeof window === "undefined") return "";
  const search = new URLSearchParams(window.location.search);
  const fromSearch = search.get(name);
  if (fromSearch) return fromSearch;
  const rawHash = window.location.hash.startsWith("#")
    ? window.location.hash.slice(1)
    : window.location.hash;
  const query = rawHash.includes("?") ? rawHash.slice(rawHash.indexOf("?") + 1) : rawHash;
  return new URLSearchParams(query).get(name) || "";
}

/** Document tabs, readers, and other secondary WebViews. */
export function isAuxiliarySurface(): boolean {
  return readSurfaceParam() !== "";
}
