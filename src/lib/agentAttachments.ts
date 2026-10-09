import { agentReadDocument, type AgentAttachment, type AgentImagePart } from "./agentApi";

export const MAX_AGENT_ATTACHMENTS = 4;
export const AGENT_ATTACHMENT_ACCEPT = "image/*,.pdf,.docx,.pptx,.xlsx,.txt,.md,.markdown,.csv,.tsv,.json,.log,.yaml,.yml";
const DOCUMENT_EXTENSIONS = new Set(["pdf", "docx", "pptx", "xlsx", "txt", "md", "markdown", "csv", "tsv", "json", "log", "yaml", "yml"]);
const MAX_IMAGE_BYTES = 10 * 1024 * 1024;
const IMAGE_MIMES = new Map(Object.entries({
  png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", gif: "image/gif",
  webp: "image/webp", bmp: "image/bmp", svg: "image/svg+xml", avif: "image/avif",
  heic: "image/heic", heif: "image/heif", tif: "image/tiff", tiff: "image/tiff",
}));

export function agentImageMime(file: Pick<File, "name" | "type">): string | null {
  const mime = file.type.toLowerCase();
  if (mime.startsWith("image/")) return mime === "image/jpg" ? "image/jpeg" : mime;
  // File/clipboard sources may omit the MIME type. Only use the filename
  // fallback when the source hasn't declared a different concrete type.
  if (mime && mime !== "application/octet-stream") return null;
  return IMAGE_MIMES.get(file.name.split(".").at(-1)?.toLowerCase() ?? "") ?? null;
}

export function isAgentImagePart(part: AgentAttachment): part is AgentImagePart {
  return "data_base64" in part;
}

export function isAgentAttachmentFile(file: File): boolean {
  return !!agentImageMime(file) || DOCUMENT_EXTENSIONS.has(file.name.split(".").at(-1)?.toLowerCase() ?? "");
}

function readBase64(file: File): Promise<string> {
  if (file.size > MAX_IMAGE_BYTES) return Promise.reject(new Error("添付が大きすぎます（1件10MBまで）"));
  if (!file.size) return Promise.reject(new Error("添付ファイルが空です"));
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    const clear = () => { reader.onload = reader.onerror = reader.onabort = null; };
    const fail = () => {
      clear();
      reject(new Error("添付ファイルを読み込めませんでした。もう一度選択してください"));
    };
    reader.onload = () => {
      const result = typeof reader.result === "string" ? reader.result : "";
      const match = /^data:[^,]*;base64,(.+)$/.exec(result);
      if (!match) { fail(); return; }
      clear();
      resolve(match[1]);
    };
    reader.onerror = reader.onabort = fail;
    try { reader.readAsDataURL(file); } catch { fail(); }
  });
}

export async function readAgentAttachment(file: File): Promise<AgentAttachment> {
  if (!isAgentAttachmentFile(file)) throw new Error("対応形式は画像、PDF、DOCX、PPTX、XLSX、TXT、Markdown、CSV、TSV、JSON、LOG、YAMLです");
  const data = await readBase64(file);
  const mime = agentImageMime(file);
  return mime ? { mime, data_base64: data } : agentReadDocument(file.name, data);
}

/** Both chat surfaces use the same limits and late-completion checks. */
export async function appendAgentAttachments(files: Iterable<File>, target: {
  active(): boolean;
  count(): number;
  append(attachment: AgentAttachment): void;
  error(message: string): void;
}): Promise<void> {
  for (const file of files) {
    if (!target.active()) return;
    if (target.count() >= MAX_AGENT_ATTACHMENTS) {
      target.error(`添付は最大${MAX_AGENT_ATTACHMENTS}件までです`);
      return;
    }
    try {
      const attachment = await readAgentAttachment(file);
      if (!target.active()) return;
      // Another picker/paste/drop may have filled the slot while reading.
      if (target.count() >= MAX_AGENT_ATTACHMENTS) {
        target.error(`添付は最大${MAX_AGENT_ATTACHMENTS}件までです`);
        return;
      }
      target.append(attachment);
    } catch (cause) {
      if (!target.active()) return;
      target.error(`${file.name}: ${cause instanceof Error ? cause.message : String(cause)}`);
    }
  }
}
