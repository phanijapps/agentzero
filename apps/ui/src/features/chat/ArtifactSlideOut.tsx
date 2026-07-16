// ============================================================================
// ARTIFACT SLIDE-OUT VIEWER
// Full-height panel sliding in from the right to preview artifact content.
// ============================================================================

import { useEffect, useState } from "react";
import { X, Download } from "lucide-react";
import { getTransport } from "@/services/transport";
import type { Artifact } from "@/services/transport/types";
import { getArtifactIcon, formatFileSize, formatJson, CsvTable } from "./artifact-utils";
import { Markdown } from "../shared/markdown";

interface ArtifactSlideOutProps {
  artifact: Artifact;
  onClose: () => void;
}

export function ArtifactSlideOut({ artifact, onClose }: ArtifactSlideOutProps) {
  const [content, setContent] = useState<string | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [contentUrl, setContentUrl] = useState("");
  const [tooLarge, setTooLarge] = useState(false);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    async function load() {
      setLoading(true);
      setContent(null);
      setPreviewError(null);
      setTooLarge(false);
      const transport = await getTransport();
      const url = transport.getArtifactContentUrl(artifact.id, artifact.sessionId);
      setContentUrl(url);

      const fileType = artifact.fileType || "";
      const textTypes = ["md", "txt", "html", "htm", "svg", "csv", "json",
        "rs", "py", "js", "ts", "tsx", "jsx", "toml", "yaml", "yml",
        "xml", "sql", "sh", "bash", "css", "go", "java", "c", "cpp", "h"];

      try {
        // Probe every type before using a browser-native preview so 413 and
        // ownership/confinement failures always produce one safe unavailable
        // state, rather than a broken image/media/embed element.
        const resp = await fetch(url);
        if (!resp.ok) {
          if (!cancelled) {
            if (resp.status === 413) {
              setTooLarge(true);
              setPreviewError("This artifact is too large to preview or download safely.");
            } else {
              setPreviewError(`Unable to load this artifact (HTTP ${resp.status}).`);
            }
          }
          return;
        }
        if (textTypes.includes(fileType)) {
          const text = await resp.text();
          if (!cancelled) setContent(text);
        }
      } catch (e) {
        console.error("Failed to preview artifact:", e);
        if (!cancelled) setPreviewError(e instanceof Error ? e.message : "Unable to preview this file");
      } finally {
        if (!cancelled) setLoading(false);
      }
    }
    load();
    return () => { cancelled = true; };
  }, [artifact.id, artifact.fileType, artifact.sessionId]);

  // Close on Escape
  useEffect(() => {
    function handleKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [onClose]);

  return (
    <>
      <div className="artifact-slideout__backdrop" onClick={onClose} role="button" tabIndex={0} onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") onClose(); }} />
      <div className="artifact-slideout">
        <div className="artifact-slideout__header">
          <div className="artifact-slideout__title">
            <span className="artifact-slideout__icon">{getArtifactIcon(artifact.fileType, 16)}</span>
            <span>{artifact.label || artifact.fileName}</span>
            <span className="artifact-slideout__meta">{artifact.fileName} · {formatFileSize(artifact.fileSize)}</span>
          </div>
          <div className="artifact-slideout__actions">
            {!tooLarge && contentUrl && (
              <a href={contentUrl} download={artifact.fileName} className="btn btn--ghost btn--sm" title="Download">
                <Download size={14} />
              </a>
            )}
            <button className="btn btn--ghost btn--sm" onClick={onClose} title="Close">
              <X size={14} />
            </button>
          </div>
        </div>
        <div className="artifact-slideout__body">
          {loading ? (
            <div style={{ display: "flex", justifyContent: "center", padding: "40px" }}>
              <span className="loading-spinner" />
            </div>
          ) : (
            renderContent(artifact, content, contentUrl, previewError, tooLarge)
          )}
        </div>
      </div>
    </>
  );
}

function renderContent(
  artifact: Artifact,
  content: string | null,
  contentUrl: string,
  previewError: string | null,
  tooLarge: boolean,
) {
  const ft = artifact.fileType || "";

  if (previewError) {
    return <PreviewUnavailable fileType={ft} fileName={artifact.fileName} contentUrl={contentUrl} error={previewError} allowDownload={!tooLarge} />;
  }

  if (ft === "md") return <Markdown className="artifact-slideout__md">{content ?? ""}</Markdown>;
  if (ft === "txt") return <pre className="artifact-slideout__pre">{content}</pre>;
  if (["html", "htm", "svg"].includes(ft)) return <iframe srcDoc={content || ""} style={{ width: "100%", height: "100%", border: "none" }} sandbox="" title="Artifact preview" />;
  if (ft === "csv") return <CsvTable content={content || ""} />;
  if (ft === "json") return <pre className="artifact-slideout__pre">{formatJson(content || "")}</pre>;
  if (["rs", "py", "js", "ts", "tsx", "jsx", "toml", "yaml", "yml", "xml", "sql", "sh", "css", "go", "java", "c", "cpp", "h"].includes(ft)) {
    return <pre className="artifact-slideout__pre"><code>{content}</code></pre>;
  }
  if (["png", "jpg", "jpeg", "gif"].includes(ft)) {
    return <img src={contentUrl} alt={artifact.fileName} style={{ maxWidth: "100%", maxHeight: "80vh", objectFit: "contain" }} />;
  }
  if (["mp4", "webm"].includes(ft)) return <video src={contentUrl} controls style={{ maxWidth: "100%" }}><track kind="captions" /></video>;
  if (["mp3", "wav"].includes(ft)) return <audio src={contentUrl} controls style={{ width: "100%" }}><track kind="captions" /></audio>;
  if (ft === "pdf") return <embed src={contentUrl} type="application/pdf" width="100%" height="100%" />;
  if (["docx", "xlsx", "pptx"].includes(ft)) {
    return <PreviewUnavailable fileType={ft} fileName={artifact.fileName} contentUrl={contentUrl} error="Office previews are disabled for safety. Download this artifact to open it locally." />;
  }
  return <PreviewUnavailable fileType={ft} fileName={artifact.fileName} contentUrl={contentUrl} />;
}

function PreviewUnavailable({
  fileType,
  fileName,
  contentUrl,
  error,
  allowDownload = true,
}: {
  fileType: string;
  fileName: string;
  contentUrl: string;
  error?: string | null;
  allowDownload?: boolean;
}) {
  return (
    <div className="artifact-slideout__empty">
      <p>{error ? `Preview failed: ${error}` : `Preview not available for .${fileType} files`}</p>
      {allowDownload && contentUrl && (
        <a href={contentUrl} download={fileName} className="btn btn--outline btn--sm">
          <Download size={14} /> Download {fileName}
        </a>
      )}
    </div>
  );
}
