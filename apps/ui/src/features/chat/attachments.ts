// =============================================================================
// Attachment helpers — compose a markdown block that hands uploaded file
// metadata (including the absolute server-side path) to the agent.
//
// All composer surfaces (chat-v2, research-v2, mission-control hero) upload
// files to /api/upload, which writes them under the vault temp directory and
// returns absolute paths. The agent only learns those paths if we splice them
// into the user prompt — there is no separate "attachments" channel on
// executeAgent today. Keeping the format identical across composers means the
// agent sees one shape regardless of where the message came from.
// =============================================================================

import type { UploadedFile } from "./ChatInput";

/** Attachment metadata safe to show in a user-message bubble. */
export interface MessageAttachment {
  name: string;
  mimeType: string;
  sizeLabel: string;
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** Convert upload metadata into the non-sensitive shape shown in chat history. */
export function displayAttachments(
  attachments: readonly UploadedFile[],
): MessageAttachment[] {
  return attachments.map((attachment) => ({
    name: attachment.name,
    mimeType: attachment.mimeType,
    sizeLabel: formatSize(attachment.size),
  }));
}

/**
 * Separate a persisted attachment table from the user-visible message text.
 * The path remains in the prompt sent to the agent, but is deliberately never
 * rendered in the chat bubble.
 */
export function splitMessageAttachments(text: string): {
  content: string;
  attachments: MessageAttachment[];
} {
  const marker = "\n\n**Attached files:**\n";
  const markerIndex = text.indexOf(marker);
  if (markerIndex === -1) return { content: text, attachments: [] };

  const rows = text.slice(markerIndex + marker.length).split("\n");
  if (rows.length < 3 || rows[0] !== "| File | Type | Size | Path |") {
    return { content: text, attachments: [] };
  }

  const attachments = rows.slice(2).flatMap((row) => {
    const cells = row.split("|").slice(1, -1).map((cell) => cell.trim());
    if (cells.length !== 4 || cells.some((cell) => cell.length === 0)) return [];
    const [name, mimeType, sizeLabel] = cells;
    return [{ name, mimeType, sizeLabel }];
  });

  return attachments.length > 0
    ? { content: text.slice(0, markerIndex), attachments }
    : { content: text, attachments: [] };
}

/**
 * Append a `**Attached files:**` markdown table to `text` listing each
 * upload's name, MIME type, size, and absolute path. Returns the original
 * text unchanged when `attachments` is empty.
 */
export function composeMessageWithAttachments(
  text: string,
  attachments: readonly UploadedFile[],
): string {
  const trimmed = text.trim();
  if (attachments.length === 0) return trimmed;
  const header = "| File | Type | Size | Path |";
  const sep = "|------|------|------|------|";
  const rows = attachments
    .map((a) => `| ${a.name} | ${a.mimeType} | ${formatSize(a.size)} | ${a.path} |`)
    .join("\n");
  return `${trimmed}\n\n**Attached files:**\n${header}\n${sep}\n${rows}`;
}
