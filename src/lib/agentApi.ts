import { invoke } from "@tauri-apps/api/core";

/** Selah agent conversation commands. */

function _isDemo(): boolean {
  try { return localStorage.getItem("selah-demo-mode") === "1"; } catch { return false; }
}

// ── Agent (Selah) ──

export interface AgentConversationSummary {
  id: string;
  title: string;
  created_at: number;
  updated_at: number;
}

export interface AgentImagePart {
  mime: string;
  data_base64: string;
}
export interface AgentDocumentPart {
  name: string;
  mime: string;
  size: number;
  text: string;
  truncated: boolean;
}
export type AgentAttachment = AgentImagePart | AgentDocumentPart;

export interface AgentMessage {
  id: number;
  conv_id: string;
  role: "user" | "assistant" | "tool";
  content: string;
  images?: AgentImagePart[] | null;
  documents?: AgentDocumentPart[];
  tool_name?: string | null;
  tool_result?: unknown;
  created_at: number;
}

export type AgentStreamEvent = (
  | { type: "phase"; stage: "planning" | "answering" }
  | { type: "plan"; steps: { name: string; detail?: string | null }[] }
  | { type: "tool_call"; name: string }
  | { type: "tool_result"; name: string; preview: string; ok: boolean }
  | { type: "think"; text: string }
  | { type: "token"; text: string }
  | { type: "done" }
  | { type: "error"; message: string }
) & { turn_id?: string };

export async function agentListConversations(): Promise<AgentConversationSummary[]> {
  if (_isDemo()) return [];
  return invoke<AgentConversationSummary[]>("agent_list_conversations");
}

export async function agentCreateConversation(title?: string): Promise<string> {
  if (_isDemo()) throw new Error("デモモードでは Agent は利用できません");
  return invoke<string>("agent_create_conversation", { title: title ?? null });
}

export async function agentLoadMessages(convId: string): Promise<AgentMessage[]> {
  if (_isDemo()) return [];
  return invoke<AgentMessage[]>("agent_load_messages", { convId });
}

/** All user/assistant messages and attachments, without hidden tool history. */
export async function agentLoadDisplayMessages(convId: string): Promise<AgentMessage[]> {
  if (_isDemo()) return [];
  return invoke<AgentMessage[]>("agent_load_display_messages", { convId });
}

export async function agentSend(
  convId: string,
  content: string,
  images: AgentImagePart[] = [],
  turnId?: string,
  documents: AgentDocumentPart[] = [],
): Promise<void> {
  if (_isDemo()) throw new Error("デモモードでは Agent は利用できません");
  return invoke("agent_send", { convId, content, images, turnId: turnId ?? null, ...(documents.length ? { documents } : {}) });
}

export function agentReadDocument(name: string, dataBase64: string): Promise<AgentDocumentPart> {
  return invoke("agent_read_document_attachment", { name, dataBase64 });
}

export async function agentCancel(convId: string, turnId?: string | null): Promise<void> {
  if (_isDemo()) return;
  return invoke("agent_cancel", { convId, turnId: turnId ?? null });
}

export async function agentDeleteConversation(convId: string): Promise<void> {
  if (_isDemo()) return;
  return invoke("agent_delete_conversation", { convId });
}

export async function agentRenameConversation(convId: string, title: string): Promise<void> {
  if (_isDemo()) return;
  return invoke("agent_rename_conversation", { convId, title });
}
