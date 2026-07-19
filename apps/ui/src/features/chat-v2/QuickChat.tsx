import { useRef, useEffect, useState } from "react";
import { Paperclip, Square, Trash2 } from "lucide-react";
import { Markdown } from "../shared/markdown";
import { ChatInput } from "../chat/ChatInput";
import { StatusPill } from "../shared/statusPill";
import { ArtifactSlideOut } from "../chat/ArtifactSlideOut";
import { getArtifactIcon } from "../chat/artifact-utils";
import { InlineActivityChip } from "./InlineActivityChip";
import { useQuickChat } from "./useQuickChat";
import { CopyButton } from "../shared/copyButton";
import type { QuickChatArtifactRef, QuickChatMessage } from "./types";
import type { Artifact } from "@/services/transport/types";
import { A2uiSurfaceRenderer } from "../surfaces/A2uiSurfaceRenderer";
import "./quick-chat.css";

function AssistantBubble({ message }: { message: QuickChatMessage }) {
  return (
    <div className="quick-chat__assistant">
      <Markdown>{message.content}</Markdown>
      {message.chips && message.chips.length > 0 && (
        <div className="quick-chat__chips">
          {message.chips.map((c) => <InlineActivityChip key={c.id} chip={c} />)}
        </div>
      )}
    </div>
  );
}

function MessageRow({ message }: { message: QuickChatMessage }) {
  const label = message.role === "user" ? "Copy question" : "Copy answer";
  return (
    <div
      className={`quick-chat__msg quick-chat__msg--${message.role}`}
      data-copy-host="true"
    >
      {message.role === "user"
        ? (
          <div className="quick-chat__user-bubble">
            <div>{message.content}</div>
            {message.attachments && message.attachments.length > 0 && (
              <div className="quick-chat__attachments" aria-label="Attachments">
                {message.attachments.map((attachment) => (
                  <span
                    className="quick-chat__attachment"
                    data-testid="quick-chat-attachment"
                    key={`${attachment.name}-${attachment.sizeLabel}`}
                    title={`${attachment.mimeType} · ${attachment.sizeLabel}`}
                  >
                    <Paperclip size={12} aria-hidden="true" />
                    {attachment.name}
                  </span>
                ))}
              </div>
            )}
          </div>
        )
        : <AssistantBubble message={message} />}
      <CopyButton text={message.content} label={label} />
    </div>
  );
}

function EmptyState({ wardName }: { wardName: string | null }) {
  return (
    <div className="quick-chat__empty">
      <h1>Quick chat</h1>
      <p className="quick-chat__empty-subtext">
        memory-aware · single-step delegation
        {wardName ? ` · bound to ${wardName}` : ""}
      </p>
    </div>
  );
}

interface ArtifactCardProps {
  artifact: QuickChatArtifactRef;
  onOpen(ref: QuickChatArtifactRef): void;
}

function ArtifactCard({ artifact, onOpen }: ArtifactCardProps) {
  return (
    <button
      type="button"
      className="quick-chat__artifact-card"
      data-testid="quick-chat-artifact"
      onClick={() => onOpen(artifact)}
      title={artifact.fileName}
    >
      <span className="quick-chat__artifact-icon" aria-hidden="true">
        {getArtifactIcon(artifact.fileType, 14)}
      </span>
      <span className="quick-chat__artifact-name">{artifact.fileName}</span>
      {artifact.label && (
        <span className="quick-chat__artifact-label">{artifact.label}</span>
      )}
    </button>
  );
}

/**
 * Lightweight ref → full Artifact shim. `ArtifactSlideOut` expects the
 * full shape (sessionId + createdAt) but for the preview we
 * only need id / fileName / fileType; the slide-out re-fetches via the
 * /artifacts/:id/content URL so the stub fields are never read.
 */
function refToArtifact(ref: QuickChatArtifactRef, sessionId: string): Artifact {
  return {
    id: ref.id,
    sessionId,
    filePath: ref.fileName,
    fileName: ref.fileName,
    fileType: ref.fileType,
    fileSize: ref.fileSize,
    label: ref.label,
    createdAt: "",
  };
}

const CLEAR_CONFIRM =
  "Clear this chat and start a new session? Past messages remain in Logs.";

export function QuickChat() {
  const { state, pillState, surfaces, sendMessage, stopAgent, clearSession } = useQuickChat();
  const endRef = useRef<HTMLDivElement | null>(null);
  const [viewing, setViewing] = useState<Artifact | null>(null);

  useEffect(() => {
    endRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [state.messages.length]);

  const hasMessages = state.messages.length > 0;
  const hasArtifacts = state.artifacts.length > 0;

  const handleClear = () => {
    if (window.confirm(CLEAR_CONFIRM)) {
      clearSession().catch(() => {});
    }
  };

  const openArtifact = (ref: QuickChatArtifactRef) => {
    if (!state.sessionId) return;
    setViewing(refToArtifact(ref, state.sessionId));
  };

  return (
    <div className="quick-chat">
      <div className="quick-chat__header">
        <div className="quick-chat__ward">
          {state.activeWardName && (
            <span className="quick-chat__ward-chip">{state.activeWardName}</span>
          )}
        </div>
        <div className="quick-chat__actions">
          {state.status === "running" && (
            <button type="button" className="btn btn--ghost btn--sm" onClick={stopAgent} title="Stop">
              <Square size={14} />
            </button>
          )}
          <button
            type="button"
            className="btn btn--ghost btn--sm"
            onClick={handleClear}
            title="Clear chat & start fresh"
            aria-label="Clear chat"
            disabled={!state.sessionId}
          >
            <Trash2 size={14} />
          </button>
        </div>
      </div>

      <div className="quick-chat__pill-strip">
        <StatusPill state={pillState} />
      </div>

      {hasMessages ? (
        <div className="quick-chat__scroll">
          <div className="quick-chat__messages">
            {state.messages.map((m) => <MessageRow key={m.id} message={m} />)}
            {(surfaces ?? []).map(surface => <A2uiSurfaceRenderer key={surface.surface_id} surface={surface} />)}
            {hasArtifacts && (
              <section className="quick-chat__deliverables" aria-labelledby="quick-chat-deliverables-heading">
                <div className="quick-chat__deliverables-heading">
                  <h2 id="quick-chat-deliverables-heading">Deliverables</h2>
                  <span>Up to 24 final outputs per chat session</span>
                </div>
                <div className="quick-chat__artifacts" data-testid="quick-chat-artifacts">
                  {state.artifacts.map((a) => (
                    <ArtifactCard key={a.id} artifact={a} onOpen={openArtifact} />
                  ))}
                </div>
              </section>
            )}
            <div ref={endRef} />
          </div>
        </div>
      ) : (
        <EmptyState wardName={state.activeWardName} />
      )}

      <div className="quick-chat__composer">
        <ChatInput
          onSend={sendMessage}
          disabled={state.status === "running" || !state.sessionId}
        />
      </div>

      {viewing && (
        <ArtifactSlideOut artifact={viewing} onClose={() => setViewing(null)} />
      )}
    </div>
  );
}
