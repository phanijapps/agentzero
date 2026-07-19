import { X } from "lucide-react";
import { FileIcon, VaultFilePreviewContent } from "./VaultFilePreview";
import type { SelectedVaultFileState } from "./useVaultFilePreview";

export function VaultFileSlideOut({
  selected,
  onClose,
}: {
  selected: SelectedVaultFileState;
  onClose: () => void;
}) {
  return (
    <>
      <div
        className="artifact-slideout__backdrop"
        onClick={onClose}
        role="button"
        tabIndex={0}
        onKeyDown={(event) => {
          if (event.key === "Enter" || event.key === " ") onClose();
        }}
      />
      <div className="artifact-slideout vault-file-slideout" role="dialog" aria-modal="true" aria-label="Vault file preview">
        <div className="artifact-slideout__header">
          <div className="artifact-slideout__title">
            <span className="artifact-slideout__icon">
              <FileIcon extension={selected.node.extension ?? ""} />
            </span>
            <span>{selected.node.name}</span>
            <span className="artifact-slideout__meta">{selected.node.path}</span>
          </div>
          <div className="artifact-slideout__actions">
            <button className="icon-btn" onClick={onClose} aria-label="Close preview">
              <X size={18} />
            </button>
          </div>
        </div>
        <div className="artifact-slideout__body">
          <VaultFilePreviewContent selected={selected} />
        </div>
      </div>
    </>
  );
}
