import { Code2, File, FileText, FolderOpen, Presentation } from "lucide-react";
import type {
  VaultTextFileResponse,
  VaultWard,
} from "@/services/transport/types";
import { Markdown } from "../shared/markdown";
import type { SelectedVaultFileState } from "./useVaultFilePreview";

export function FileIcon({ extension }: { extension: string }) {
  if (extension === "md" || extension === "txt" || extension === "docx") return <FileText size={16} />;
  if (extension === "pptx" || extension === "ppt") return <Presentation size={16} />;
  if (["py", "js", "ts", "tsx", "html", "css", "json", "toml", "yaml", "yml"].includes(extension)) {
    return <Code2 size={16} />;
  }
  return <File size={16} />;
}

export function VaultFilePreviewPane({
  selected,
  ward,
  directoryCount,
  fileCount,
  onOpenWard,
}: {
  selected: SelectedVaultFileState | null;
  ward: VaultWard;
  directoryCount: number;
  fileCount: number;
  onOpenWard: () => Promise<void>;
}) {
  if (!selected) {
    return (
      <section className="vault-preview-pane" aria-label="Vault file preview">
        <div className="vault-empty-preview">
          <div className="vault-empty-preview__glyph" aria-hidden="true">
            <FolderOpen size={34} />
          </div>
          <div>
            <p className="vault-empty-preview__eyebrow">Ward content</p>
            <p className="vault-state">{ward.name}</p>
            <p className="vault-empty-preview__meta">{directoryCount} dirs / {fileCount} files loaded</p>
          </div>
        </div>
      </section>
    );
  }

  const { node } = selected;
  return (
    <section className="vault-preview-pane" aria-label="Vault file preview">
      <header className="vault-preview-pane__head">
        <div>
          <h2>{node.name}</h2>
          <p>{node.path}</p>
        </div>
        {!node.previewable || selected.content?.kind === "office" ? (
          <button className="btn btn--outline btn--sm" type="button" onClick={() => void onOpenWard()}>
            <FolderOpen size={14} />
            Open ward folder
          </button>
        ) : null}
      </header>
      <div className="vault-preview-pane__body">
        <VaultFilePreviewContent selected={selected} />
      </div>
    </section>
  );
}

export function VaultFilePreviewContent({ selected }: { selected: SelectedVaultFileState }) {
  const { node, content } = selected;
  if (selected.loading) return <p className="vault-state">Loading preview...</p>;
  if (selected.error) return <p className="vault-state vault-state--error">{selected.error}</p>;
  if (!node.previewable) {
    return <p className="vault-state">Preview not available for .{node.extension} files.</p>;
  }
  if (!content) return null;
  if (content.kind === "text") return <TextPreview file={content} />;
  return <p className="vault-state">Preview not available.</p>;
}

function TextPreview({ file }: { file: VaultTextFileResponse }) {
  if (file.extension === "md") {
    return <Markdown className="artifact-slideout__md vault-markdown">{file.content}</Markdown>;
  }
  if (file.extension === "html") {
    return (
      <iframe
        className="vault-html-preview"
        sandbox=""
        srcDoc={file.content}
        title={`HTML preview: ${file.name}`}
      />
    );
  }
  return (
    <pre className="vault-code-preview">
      <code>{file.content}</code>
    </pre>
  );
}
