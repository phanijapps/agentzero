// ============================================================================
// HERO INPUT
// Beautiful centered input for new sessions — the landing experience.
// Shows when there are no blocks and status is idle.
// On send, triggers the same sendMessage as MissionControl's ChatInput.
// ============================================================================

import { useState, useRef, useCallback } from "react";
import { Paperclip, ArrowUp, CheckCircle2, XCircle } from "lucide-react";
import type { UploadedFile } from "./ChatInput";
import type { LogSession } from "@/services/transport/types";
import { timeAgo, switchToSession } from "./mission-hooks";

// ============================================================================
// Types
// ============================================================================

interface HeroInputProps {
  onSend: (message: string, attachments: UploadedFile[]) => void;
  recentSessions?: LogSession[];
  /** Override how a recent-session card click is handled. Defaults to
   *  switching the /chat (mission-control) session. research-v2 passes a
   *  router-based handler so the card opens under /research-v2/:id. */
  onSelectSession?: (sessionId: string, conversationId: string) => void;
}

// ============================================================================
// File Upload Helper (same as ChatInput)
// ============================================================================

async function uploadFile(file: File): Promise<UploadedFile> {
  const form = new FormData();
  form.append("file", file);
  const res = await fetch("/api/upload", { method: "POST", body: form });
  if (!res.ok) {
    throw new Error(`Upload failed: ${res.statusText}`);
  }
  const data = await res.json();
  return {
    id: data.id,
    name: data.filename,
    mimeType: data.mime_type,
    size: data.size,
    path: data.path,
  };
}

// ============================================================================
// Suggestions
// ============================================================================

const SUGGESTIONS = [
  "Analyze recent trends",
  "Write a report",
  "Debug an issue",
  "Summarize a document",
];

// ============================================================================
// Component
// ============================================================================

export function HeroInput({ onSend, recentSessions = [], onSelectSession }: HeroInputProps) {
  const handleSelectSession = onSelectSession ?? switchToSession;
  const [text, setText] = useState("");
  const [attachments, setAttachments] = useState<UploadedFile[]>([]);
  const [uploading, setUploading] = useState(false);
  const [uploadError, setUploadError] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const canSend = text.trim().length > 0 || attachments.length > 0;
  const isDisabled = uploading;

  const handleSend = useCallback(() => {
    if (!canSend || isDisabled) return;
    onSend(text.trim(), attachments);
    setText("");
    setAttachments([]);
  }, [canSend, isDisabled, text, attachments, onSend]);

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey && !e.repeat) {
      e.preventDefault();
      handleSend();
    }
  };

  const handleFileSelect = async (files: FileList | null) => {
    if (!files || files.length === 0) return;
    setUploading(true);
    setUploadError(null);
    try {
      const uploaded = await Promise.all(
        Array.from(files).map((f) => uploadFile(f)),
      );
      setAttachments((prev) => [...prev, ...uploaded]);
    } catch (err) {
      const msg = err instanceof Error ? err.message : "Upload failed";
      console.error("File upload failed:", err);
      setUploadError(msg);
      setTimeout(() => setUploadError(null), 5000);
    } finally {
      setUploading(false);
    }
  };

  const removeAttachment = (id: string) => {
    setAttachments((prev) => prev.filter((a) => a.id !== id));
  };

  const handleDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    handleFileSelect(e.dataTransfer.files);
  }, []);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
  }, []);

  const handleSuggestionClick = (suggestion: string) => {
    setText(suggestion);
    textareaRef.current?.focus();
  };

  return (
    <div className="hero-input" onDrop={handleDrop} onDragOver={handleDragOver} role="region" aria-label="Drop files here">
      {/* Brand */}
      <div className="hero-input__brand">
        <span className="hero-input__logo" role="img" aria-label="z-Bot">z</span>
        <span className="hero-input__name">z-Bot</span>
      </div>

      {/* Input container */}
      <div className="hero-input__container">
        {/* Pending attachment chips */}
        {attachments.length > 0 && (
          <div className="hero-input__chips">
            {attachments.map((a) => (
              <span key={a.id} className="chat-input__chip">
                {a.name}
                <span
                  className="chat-input__chip-remove"
                  onClick={() => removeAttachment(a.id)}
                  role="button"
                  tabIndex={0}
                  onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") removeAttachment(a.id); }}
                >
                  x
                </span>
              </span>
            ))}
          </div>
        )}

        <textarea
          ref={textareaRef}
          className="hero-input__field"
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="What would you like to work on?"
          disabled={isDisabled}
          rows={1}
        />

        {/* Action buttons inside the field */}
        <div className="hero-input__actions">
          <button
            className="hero-input__action-btn"
            title="Attach file"
            onClick={() => fileInputRef.current?.click()}
            disabled={isDisabled}
          >
            <Paperclip style={{ width: 18, height: 18 }} />
          </button>
          <input
            ref={fileInputRef}
            type="file"
            hidden
            multiple
            onChange={(e) => {
              handleFileSelect(e.target.files);
              e.target.value = "";
            }}
          />

          <button
            className="hero-input__send"
            onClick={handleSend}
            disabled={!canSend || isDisabled}
            title="Send message"
          >
            <ArrowUp style={{ width: 18, height: 18 }} />
          </button>
        </div>
      </div>

      {uploading && (
        <div className="chat-input__uploading" style={{ marginTop: 8 }}>
          Uploading...
        </div>
      )}
      {uploadError && (
        <div style={{ marginTop: 8, fontSize: "var(--text-xs)", color: "var(--destructive)", textAlign: "center" }}>
          {uploadError}
        </div>
      )}

      {/* Quick-action suggestions */}
      <div className="hero-input__suggestions">
        {SUGGESTIONS.map((s) => (
          <button
            key={s}
            className="hero-input__suggestion"
            onClick={() => handleSuggestionClick(s)}
          >
            {s}
          </button>
        ))}
      </div>

      {/* Recent sessions */}
      {recentSessions.length > 0 && (
        <div className="hero-input__recent">
          <span className="hero-input__recent-label">Recent</span>
          <div className="hero-input__recent-cards">
            {recentSessions.slice(0, 3).map((s) => {
              const displayTitle = s.title?.slice(0, 40) || "Untitled";
              const isOk = s.status === "completed";
              return (
                <button
                  key={s.session_id}
                  className="hero-input__recent-card"
                  onClick={() => handleSelectSession(s.session_id, s.conversation_id)}
                >
                  <span className="hero-input__recent-title">{displayTitle}</span>
                  <span className="hero-input__recent-meta">
                    {isOk
                      ? <CheckCircle2 style={{ width: 12, height: 12, color: "var(--success)" }} />
                      : <XCircle style={{ width: 12, height: 12, color: "var(--destructive)" }} />}
                    <span className="hero-input__recent-time">{timeAgo(s.started_at)}</span>
                  </span>
                </button>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}
