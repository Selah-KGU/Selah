import { invoke } from "@tauri-apps/api/core";

export interface ChatGptAccount {
  client_id: string;
  email: string;
  active: boolean;
  connected: boolean;
  plan_enabled: boolean;
}

export interface ChatGptModel {
  slug: string;
  display_name: string;
}

export function getChatGptAccounts(): Promise<ChatGptAccount[]> {
  return invoke("chatgpt_accounts");
}

export async function isChatGptReady(): Promise<boolean> {
  const accounts = await getChatGptAccounts();
  return accounts.some((account) => account.active && account.connected && account.plan_enabled);
}
