export type DownloadEvent =
  | { event: "Started"; data: { contentLength?: number } }
  | { event: "Progress"; data: { chunkLength: number } }
  | { event: "Finished" };

export interface RuntimeUpdate {
  version: string;
  body?: string;
  close(): Promise<void>;
  downloadAndInstall(
    onEvent?: (progress: DownloadEvent) => void,
    options?: { timeout?: number }
  ): Promise<void>;
}

export async function checkForRuntimeUpdate(): Promise<RuntimeUpdate | null> {
  throw new Error("This action is not available in this distribution.");
}
