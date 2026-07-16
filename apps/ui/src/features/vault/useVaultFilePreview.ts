import { useEffect, useRef, useState } from "react";
import { getTransport } from "@/services/transport";
import type { VaultFileResponse, VaultNode } from "@/services/transport/types";

export interface SelectedVaultFileState {
  node: VaultNode;
  content: VaultFileResponse | null;
  loading: boolean;
  error: string | null;
}

export function useVaultFilePreview(wardId: string | null) {
  const [selectedFile, setSelectedFile] = useState<SelectedVaultFileState | null>(null);
  const requestRef = useRef(0);
  const wardRef = useRef(wardId);

  useEffect(() => {
    wardRef.current = wardId;
    requestRef.current += 1;
    setSelectedFile(null);
  }, [wardId]);

  async function selectFile(node: VaultNode) {
    if (!wardId) return;
    const request = requestRef.current + 1;
    requestRef.current = request;
    const initial: SelectedVaultFileState = {
      node,
      content: null,
      loading: node.previewable,
      error: null,
    };
    setSelectedFile(initial);
    if (!node.previewable) return;

    const transport = await getTransport();
    const result = await transport.getVaultFile(wardId, node.path);
    if (wardRef.current !== wardId || requestRef.current !== request) return;
    if (!result.success || !result.data) {
      setSelectedFile({ ...initial, loading: false, error: result.error ?? "Failed to load file" });
      return;
    }

    if (result.data.kind === "office") {
      // Ward contents are agent-writable and therefore untrusted. Do not
      // decompress Office ZIP containers in the browser; callers can open the
      // local ward folder to use a trusted desktop viewer instead.
      setSelectedFile({
        ...initial,
        content: result.data,
        loading: false,
        error: "Office previews are disabled for safety. Open the ward folder to view this file locally.",
      });
      return;
    }

    setSelectedFile({ node, content: result.data, loading: false, error: null });
  }

  function clearSelectedFile() {
    requestRef.current += 1;
    setSelectedFile(null);
  }

  return { selectedFile, selectFile, clearSelectedFile };
}
