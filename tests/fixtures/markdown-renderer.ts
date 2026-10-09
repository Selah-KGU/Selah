import { marked } from "marked";
export { createMarkdownRenderer } from "../../src/lib/markdownRenderer";

export function globalLineBreaks(enabled: boolean): void {
  marked.setOptions({ breaks: enabled });
}
export function renderWithGlobalParser(text: string): string {
  return marked.parse(text, { async: false });
}
