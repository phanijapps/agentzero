import { describeTool } from "../shared/statusPill/tool-phrase";
import type { TimelineEntry } from "./types";

/**
 * Durable, compact record of non-response tool calls for a completed turn.
 *
 * The live status pill is intentionally transient. Retaining only safe tool
 * names and their human-readable verbs lets a user verify work such as
 * `recall` after the live stream ends without exposing tool arguments or
 * results.
 */
export function ToolActivity({ entries }: { entries: TimelineEntry[] }) {
  const calls = entries.filter(
    (entry): entry is TimelineEntry & { toolName: string } =>
      entry.kind === "tool_call" &&
      typeof entry.toolName === "string" &&
      entry.toolName.length > 0 &&
      entry.toolName !== "respond",
  );

  if (calls.length === 0) return null;

  return (
    <div className="tool-activity" data-testid="turn-tool-activity" aria-label="Tools used">
      <span className="tool-activity__label">Tools used</span>
      <ul className="tool-activity__list">
        {calls.map((entry) => {
          const phrase = describeTool(entry.toolName, {});
          return (
            <li key={entry.id} className="tool-activity__item" title={phrase.narration}>
              <code>{entry.toolName}</code>
              <span>{phrase.narration}</span>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
