import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type MouseEvent as ReactMouseEvent,
} from "react";
import { useSearchParams } from "react-router-dom";
import {
  Archive,
  ChevronRight,
  Folder,
  FolderOpen,
  PanelLeftClose,
  PanelLeftOpen,
  RefreshCw,
} from "lucide-react";
import { getTransport } from "@/services/transport";
import type { VaultWard } from "@/services/transport/types";
import { WardVaultExplorer, type WardVaultRootStats } from "./WardVaultExplorer";
import { VaultFilePreviewPane } from "./VaultFilePreview";
import { useVaultFilePreview } from "./useVaultFilePreview";

type LoadState = "idle" | "loading" | "error";

const MIN_EXPLORER_WIDTH = 240;
const DEFAULT_EXPLORER_WIDTH = 340;
const MAX_EXPLORER_WIDTH = 560;
const EXPLORER_WIDTH_STEP = 24;

function clampExplorerWidth(width: number, splitElement: HTMLDivElement | null) {
  const splitWidth = splitElement?.getBoundingClientRect().width ?? 0;
  const availableMax = splitWidth > 0
    ? Math.max(MIN_EXPLORER_WIDTH, Math.min(MAX_EXPLORER_WIDTH, splitWidth - 420))
    : MAX_EXPLORER_WIDTH;
  return Math.min(availableMax, Math.max(MIN_EXPLORER_WIDTH, Math.round(width)));
}

export function VaultPage() {
  const [searchParams, setSearchParams] = useSearchParams();
  const requestedWardId = searchParams.get("ward");
  const activeSection = searchParams.get("section");
  const showingWards = activeSection === "wards";
  const [wards, setWards] = useState<VaultWard[]>([]);
  const [wardState, setWardState] = useState<LoadState>("loading");
  const [wardError, setWardError] = useState<string | null>(null);
  const [selectedWard, setSelectedWard] = useState<VaultWard | null>(null);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [explorerWidth, setExplorerWidth] = useState(DEFAULT_EXPLORER_WIDTH);
  const [isResizingExplorer, setIsResizingExplorer] = useState(false);
  const [rootStats, setRootStats] = useState<WardVaultRootStats>({ directoryCount: 0, fileCount: 0 });
  const { selectedFile, selectFile } = useVaultFilePreview(selectedWard?.id ?? null);
  const splitRef = useRef<HTMLDivElement | null>(null);
  const resizeStartRef = useRef({ clientX: 0, width: DEFAULT_EXPLORER_WIDTH });

  const loadWards = useCallback(async () => {
    setWardState("loading");
    setWardError(null);
    const transport = await getTransport();
    const result = await transport.listVaultWards();
    if (!result.success || !result.data) {
      setWardState("error");
      setWardError(result.error ?? "Failed to load wards");
      return;
    }
    setWards(result.data.wards);
    setWardState("idle");
  }, []);

  const activateWard = useCallback((ward: VaultWard) => {
    setSelectedWard(ward);
    setRootStats({ directoryCount: 0, fileCount: 0 });
  }, []);

  const resetWardExplorer = useCallback(() => {
    setSelectedWard(null);
    setRootStats({ directoryCount: 0, fileCount: 0 });
  }, []);

  useEffect(() => {
    void loadWards();
  }, [loadWards]);

  useEffect(() => {
    if (!isResizingExplorer) return;

    const previousCursor = document.body.style.cursor;
    const previousUserSelect = document.body.style.userSelect;
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";

    function handleMouseMove(event: MouseEvent) {
      const delta = event.clientX - resizeStartRef.current.clientX;
      setExplorerWidth(clampExplorerWidth(resizeStartRef.current.width + delta, splitRef.current));
    }

    function stopResizing() {
      setIsResizingExplorer(false);
    }

    window.addEventListener("mousemove", handleMouseMove);
    window.addEventListener("mouseup", stopResizing);
    return () => {
      document.body.style.cursor = previousCursor;
      document.body.style.userSelect = previousUserSelect;
      window.removeEventListener("mousemove", handleMouseMove);
      window.removeEventListener("mouseup", stopResizing);
    };
  }, [isResizingExplorer]);

  useEffect(() => {
    if (wardState !== "idle") return;
    if (!requestedWardId) {
      if (selectedWard) resetWardExplorer();
      return;
    }
    if (selectedWard?.id === requestedWardId) return;

    const ward = wards.find((item) => item.id === requestedWardId);
    if (ward) {
      void activateWard(ward);
    } else {
      resetWardExplorer();
    }
  }, [activateWard, requestedWardId, resetWardExplorer, selectedWard, wardState, wards]);

  function selectWard(ward: VaultWard) {
    setSearchParams({ ward: ward.id });
  }

  function openWards() {
    resetWardExplorer();
    setSearchParams({ section: "wards" });
  }

  async function openSelectedWardFolder() {
    if (!selectedWard) return;
    const transport = await getTransport();
    await transport.openWard(selectedWard.id);
  }

  function returnToWards() {
    openWards();
  }

  function returnToVaultRoot() {
    resetWardExplorer();
    setSearchParams({});
  }

  function beginExplorerResize(event: ReactMouseEvent<HTMLDivElement>) {
    event.preventDefault();
    resizeStartRef.current = { clientX: event.clientX, width: explorerWidth };
    setIsResizingExplorer(true);
  }

  function resizeExplorerBy(delta: number) {
    setExplorerWidth((current) => clampExplorerWidth(current + delta, splitRef.current));
  }

  function handleExplorerResizeKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "ArrowLeft") {
      event.preventDefault();
      resizeExplorerBy(-EXPLORER_WIDTH_STEP);
    } else if (event.key === "ArrowRight") {
      event.preventDefault();
      resizeExplorerBy(EXPLORER_WIDTH_STEP);
    } else if (event.key === "Home") {
      event.preventDefault();
      setExplorerWidth(MIN_EXPLORER_WIDTH);
    } else if (event.key === "End") {
      event.preventDefault();
      setExplorerWidth(clampExplorerWidth(MAX_EXPLORER_WIDTH, splitRef.current));
    }
  }

  const breadcrumb = useMemo(() => (
    selectedWard
      ? ["Vault", "Wards", selectedWard.name]
      : showingWards
        ? ["Vault", "Wards"]
        : ["Vault"]
  ), [selectedWard, showingWards]);
  const wardsExpanded = showingWards || Boolean(selectedWard);
  const headerStatus = selectedWard
    ? `${rootStats.directoryCount} dirs / ${rootStats.fileCount} files`
    : showingWards
      ? `${wards.length} wards`
      : "Vault root";

  return (
    <div className="vault-page">
      <header className="vault-page__header">
        <div className="vault-page__identity">
          <span className="vault-page__mark" aria-hidden="true">
            <Archive size={18} />
          </span>
          <nav className="vault-breadcrumb" aria-label="Breadcrumb">
            {breadcrumb.map((item, index) => (
              <span className="vault-breadcrumb__item" key={`${item}-${index}`}>
                {index > 0 ? <ChevronRight size={14} aria-hidden="true" /> : null}
                {(selectedWard || showingWards) && index === 0 ? (
                  <button
                    className="vault-breadcrumb__button"
                    type="button"
                    onClick={returnToVaultRoot}
                  >
                    {item}
                  </button>
                ) : selectedWard && index === 1 ? (
                  <button
                    className="vault-breadcrumb__button"
                    type="button"
                    onClick={returnToWards}
                  >
                    {item}
                  </button>
                ) : (
                  <span>{item}</span>
                )}
              </span>
            ))}
          </nav>
        </div>
        <div className="vault-page__actions">
          <span className="vault-status-pill">{headerStatus}</span>
          <button
            className="icon-btn"
            type="button"
            onClick={() => setSidebarCollapsed((current) => !current)}
            aria-label={sidebarCollapsed ? "Expand vault explorer" : "Collapse vault explorer"}
            aria-pressed={sidebarCollapsed}
          >
            {sidebarCollapsed ? <PanelLeftOpen size={16} /> : <PanelLeftClose size={16} />}
          </button>
          <button className="icon-btn" type="button" onClick={loadWards} aria-label="Refresh vault wards">
            <RefreshCw size={16} />
          </button>
        </div>
      </header>

      <div
        ref={splitRef}
        className={`vault-split${sidebarCollapsed ? " vault-split--collapsed" : ""}${isResizingExplorer ? " vault-split--resizing" : ""}`}
        style={{ "--vault-explorer-width": `${explorerWidth}px` } as CSSProperties}
      >
        {!sidebarCollapsed ? (
          <>
            {selectedWard ? (
              <WardVaultExplorer
                ward={selectedWard}
                selectedPath={selectedFile?.node.path ?? null}
                onSelectFile={(node) => void selectFile(node)}
                onRootStatsChange={setRootStats}
                onCollapse={() => setSidebarCollapsed(true)}
              />
            ) : (
              <aside className="vault-tree-pane" aria-label="Vault explorer">
                <div className="vault-tree-pane__head">
                  <span className="vault-tree-pane__title">Vault</span>
                  <div className="vault-tree-pane__actions">
                    <span className="vault-tree-pane__meta">
                      {wardState === "loading" ? "Loading" : `${wards.length} wards`}
                    </span>
                    <button
                      className="icon-btn icon-btn--sm"
                      type="button"
                      onClick={() => setSidebarCollapsed(true)}
                      aria-label="Collapse vault explorer"
                    >
                      <PanelLeftClose size={14} />
                    </button>
                  </div>
                </div>
                <VaultExplorer
                  wards={wards}
                  state={wardState}
                  error={wardError}
                  wardsExpanded={wardsExpanded}
                  onOpenWards={openWards}
                  onSelectWard={selectWard}
                />
              </aside>
            )}
            <div
              className="vault-split__resizer"
              role="separator"
              aria-label="Resize vault explorer"
              aria-orientation="vertical"
              aria-valuemin={MIN_EXPLORER_WIDTH}
              aria-valuemax={MAX_EXPLORER_WIDTH}
              aria-valuenow={explorerWidth}
              tabIndex={0}
              onMouseDown={beginExplorerResize}
              onKeyDown={handleExplorerResizeKeyDown}
            >
              <span aria-hidden="true" />
            </div>
          </>
        ) : null}
        {selectedWard ? (
          <VaultFilePreviewPane
            selected={selectedFile}
            ward={selectedWard}
            directoryCount={rootStats.directoryCount}
            fileCount={rootStats.fileCount}
            onOpenWard={openSelectedWardFolder}
          />
        ) : (
          <section className="vault-preview-pane" aria-label="Vault file preview">
            <EmptyPreview showingWards={showingWards} wardCount={wards.length} />
          </section>
        )}
      </div>
    </div>
  );
}

function EmptyPreview({
  showingWards,
  wardCount,
}: {
  showingWards: boolean;
  wardCount: number;
}) {
  return (
    <div className="vault-empty-preview">
      <div className="vault-empty-preview__glyph" aria-hidden="true">
        <Archive size={34} />
      </div>
      <div>
        <p className="vault-empty-preview__eyebrow">{showingWards ? `${wardCount} wards` : "Vault root"}</p>
        <p className="vault-state">No file selected.</p>
      </div>
    </div>
  );
}

function VaultExplorer({
  wards,
  state,
  error,
  wardsExpanded,
  onOpenWards,
  onSelectWard,
}: {
  wards: VaultWard[];
  state: LoadState;
  error: string | null;
  wardsExpanded: boolean;
  onOpenWards: () => void;
  onSelectWard: (ward: VaultWard) => void;
}) {
  return (
    <div className="vault-tree">
      <button
        type="button"
        aria-expanded={wardsExpanded}
        className="vault-tree-row"
        style={{ paddingLeft: 8 }}
        onClick={onOpenWards}
      >
        {wardsExpanded ? <FolderOpen size={16} /> : <Folder size={16} />}
        <span className="vault-tree-row__name">Wards</span>
      </button>
      {wardsExpanded ? (
        <>
          {state === "loading" ? <p className="vault-state vault-state--inline">Loading wards...</p> : null}
          {state === "error" ? <p className="vault-state vault-state--error vault-state--inline">{error}</p> : null}
          {state === "idle" && wards.length === 0 ? (
            <p className="vault-state vault-state--inline">No wards found.</p>
          ) : null}
          {wards.map((ward) => (
            <div key={ward.id}>
              <button
                type="button"
                aria-expanded={false}
                className="vault-tree-row"
                style={{ paddingLeft: 26 }}
                onClick={() => onSelectWard(ward)}
              >
                <Folder size={16} />
                <span className="vault-tree-row__name">{ward.name}</span>
              </button>
            </div>
          ))}
        </>
      ) : null}
    </div>
  );
}
