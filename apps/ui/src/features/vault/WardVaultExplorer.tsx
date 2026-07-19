import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { FolderOpen, Folder, PanelLeftClose, Search, X } from "lucide-react";
import { getTransport } from "@/services/transport";
import type { VaultNode, VaultWard } from "@/services/transport/types";
import { FileIcon } from "./VaultFilePreview";

type LoadState = "idle" | "loading" | "error";

export interface WardVaultRootStats {
  directoryCount: number;
  fileCount: number;
}

export function WardVaultExplorer({
  ward,
  refreshKey = 0,
  selectedPath,
  onSelectFile,
  onRootStatsChange,
  onCollapse,
  actions,
  className = "",
  ariaLabel = "Vault explorer",
}: {
  ward: VaultWard;
  refreshKey?: unknown;
  selectedPath: string | null;
  onSelectFile: (node: VaultNode) => void;
  onRootStatsChange?: (stats: WardVaultRootStats) => void;
  onCollapse?: () => void;
  actions?: ReactNode;
  className?: string;
  ariaLabel?: string;
}) {
  const [childrenByPath, setChildrenByPath] = useState<Record<string, VaultNode[]>>({});
  const [truncatedByPath, setTruncatedByPath] = useState<Record<string, boolean>>({});
  const [expanded, setExpanded] = useState<Set<string>>(new Set([""]));
  const [loadingPaths, setLoadingPaths] = useState<Set<string>>(new Set());
  const [treeError, setTreeError] = useState<string | null>(null);
  const [rootPending, setRootPending] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchState, setSearchState] = useState<LoadState>("idle");
  const [searchError, setSearchError] = useState<string | null>(null);
  const [searchResults, setSearchResults] = useState<VaultNode[]>([]);
  const [searchTruncated, setSearchTruncated] = useState(false);
  const treeGenerationRef = useRef(0);
  const searchRequestRef = useRef(0);
  const didMountRefreshRef = useRef(false);
  const childrenByPathRef = useRef(childrenByPath);
  const expandedRef = useRef(expanded);

  useEffect(() => {
    childrenByPathRef.current = childrenByPath;
  }, [childrenByPath]);

  useEffect(() => {
    expandedRef.current = expanded;
  }, [expanded]);

  const loadTree = useCallback(async (wardId: string, path: string, generation: number) => {
    setTreeError(null);
    setLoadingPaths((current) => new Set(current).add(path));
    const transport = await getTransport();
    const result = await transport.getVaultTree(wardId, path);
    if (treeGenerationRef.current !== generation) return;
    setLoadingPaths((current) => {
      const next = new Set(current);
      next.delete(path);
      return next;
    });
    if (!result.success || !result.data) {
      if (path === "" && isNotFoundError(result.error)) {
        setRootPending(true);
        setChildrenByPath((current) => ({ ...current, "": [] }));
        setTruncatedByPath((current) => ({ ...current, "": false }));
        return;
      }
      setTreeError(result.error ?? "Failed to load directory");
      return;
    }
    const data = result.data;
    if (path === "") setRootPending(false);
    setChildrenByPath((current) => ({ ...current, [path]: data.children }));
    setTruncatedByPath((current) => ({ ...current, [path]: data.truncated }));
  }, []);

  const searchVaultFiles = useCallback(async (wardId: string, query: string, request: number) => {
    const transport = await getTransport();
    const result = await transport.searchVaultFiles(wardId, query, 30);
    if (searchRequestRef.current !== request || ward.id !== wardId) return;
    if (!result.success || !result.data) {
      setSearchState("error");
      setSearchError(result.error ?? "Failed to search files");
      setSearchResults([]);
      setSearchTruncated(false);
      return;
    }
    setSearchState("idle");
    setSearchError(null);
    setSearchResults(result.data.matches);
    setSearchTruncated(result.data.truncated);
  }, [ward.id]);

  useEffect(() => {
    const generation = treeGenerationRef.current + 1;
    treeGenerationRef.current = generation;
    didMountRefreshRef.current = false;
    searchRequestRef.current += 1;
    setChildrenByPath({});
    setTruncatedByPath({});
    setExpanded(new Set([""]));
    setLoadingPaths(new Set());
    setTreeError(null);
    setRootPending(false);
    setSearchQuery("");
    setSearchState("idle");
    setSearchError(null);
    setSearchResults([]);
    setSearchTruncated(false);
    onRootStatsChange?.({ directoryCount: 0, fileCount: 0 });
    void loadTree(ward.id, "", generation);
  }, [loadTree, onRootStatsChange, ward.id]);

  useEffect(() => {
    if (!didMountRefreshRef.current) {
      didMountRefreshRef.current = true;
      return;
    }
    const generation = treeGenerationRef.current + 1;
    treeGenerationRef.current = generation;
    setTreeError(null);
    const currentChildrenByPath = childrenByPathRef.current;
    const paths = [...expandedRef.current].filter((path) => path === "" || currentChildrenByPath[path] != null);
    if (paths.length === 0) paths.push("");
    for (const path of paths) {
      void loadTree(ward.id, path, generation);
    }
  }, [loadTree, refreshKey, ward.id]);

  useEffect(() => {
    const rootChildren = childrenByPath[""] ?? [];
    onRootStatsChange?.({
      directoryCount: rootChildren.filter((child) => child.kind === "directory").length,
      fileCount: rootChildren.filter((child) => child.kind === "file").length,
    });
  }, [childrenByPath, onRootStatsChange]);

  useEffect(() => {
    const query = searchQuery.trim();
    searchRequestRef.current += 1;
    const request = searchRequestRef.current;
    if (!query) {
      setSearchState("idle");
      setSearchError(null);
      setSearchResults([]);
      setSearchTruncated(false);
      return;
    }

    setSearchState("loading");
    setSearchError(null);
    setSearchResults([]);
    setSearchTruncated(false);
    const timer = window.setTimeout(() => {
      void searchVaultFiles(ward.id, query, request);
    }, 180);
    return () => window.clearTimeout(timer);
  }, [searchQuery, searchVaultFiles, ward.id]);

  async function toggleDirectory(node: VaultNode) {
    const isOpen = expanded.has(node.path);
    setExpanded((current) => {
      const next = new Set(current);
      if (isOpen) next.delete(node.path);
      else next.add(node.path);
      return next;
    });
    if (!isOpen && childrenByPath[node.path] == null) {
      await loadTree(ward.id, node.path, treeGenerationRef.current);
    }
  }

  return (
    <aside className={`vault-tree-pane${className ? ` ${className}` : ""}`} aria-label={ariaLabel}>
      <div className="vault-tree-pane__head">
        <span className="vault-tree-pane__title">{ward.name}</span>
        <div className="vault-tree-pane__actions">
          <span className="vault-tree-pane__meta">Ward content</span>
          {actions}
          {onCollapse ? (
            <button
              className="icon-btn icon-btn--sm"
              type="button"
              onClick={onCollapse}
              aria-label="Collapse vault explorer"
            >
              <PanelLeftClose size={14} />
            </button>
          ) : null}
        </div>
      </div>
      {treeError ? <p className="vault-state vault-state--error">{treeError}</p> : null}
      <VaultSearchBox
        wardName={ward.name}
        query={searchQuery}
        state={searchState}
        error={searchError}
        results={searchResults}
        truncated={searchTruncated}
        selectedPath={selectedPath}
        onQueryChange={setSearchQuery}
        onClear={() => setSearchQuery("")}
        onSelectFile={onSelectFile}
      />
      <div className="vault-tree">
        <button
          type="button"
          aria-expanded
          aria-current={selectedPath == null ? "page" : undefined}
          className={`vault-tree-row${selectedPath == null ? " vault-tree-row--selected" : ""}`}
          style={{ paddingLeft: 8 }}
        >
          <FolderOpen size={16} />
          <span className="vault-tree-row__name">{ward.name}</span>
        </button>
        <WardTree
          nodes={childrenByPath[""] ?? []}
          childrenByPath={childrenByPath}
          truncatedByPath={truncatedByPath}
          expanded={expanded}
          loadingPaths={loadingPaths}
          selectedPath={selectedPath}
          onToggle={(node) => void toggleDirectory(node)}
          onSelectFile={onSelectFile}
          depth={1}
        />
        {rootPending ? (
          <p className="vault-state vault-state--inline">Waiting for ward files...</p>
        ) : null}
        {truncatedByPath[""] ? (
          <p className="vault-state vault-state--inline">Directory truncated at 1,000 entries.</p>
        ) : null}
      </div>
    </aside>
  );
}

function isNotFoundError(error: string | undefined): boolean {
  if (!error) return false;
  return error.includes("HTTP 404") || error.toLowerCase().includes("not found");
}

function VaultSearchBox({
  wardName,
  query,
  state,
  error,
  results,
  truncated,
  selectedPath,
  onQueryChange,
  onClear,
  onSelectFile,
}: {
  wardName: string;
  query: string;
  state: LoadState;
  error: string | null;
  results: VaultNode[];
  truncated: boolean;
  selectedPath: string | null;
  onQueryChange: (query: string) => void;
  onClear: () => void;
  onSelectFile: (node: VaultNode) => void;
}) {
  const hasQuery = query.trim().length > 0;
  return (
    <section className="vault-search" aria-label="Vault file search">
      <div className="vault-search__field">
        <Search size={14} aria-hidden="true" />
        <input
          type="search"
          value={query}
          onChange={(event) => onQueryChange(event.currentTarget.value)}
          placeholder={`Search ${wardName}`}
          aria-label={`Fuzzy search files in ${wardName}`}
        />
        {hasQuery ? (
          <button className="vault-search__clear" type="button" onClick={onClear} aria-label="Clear file search">
            <X size={14} />
          </button>
        ) : null}
      </div>
      {state === "loading" ? <p className="vault-state vault-state--inline">Searching files...</p> : null}
      {state === "error" ? <p className="vault-state vault-state--error vault-state--inline">{error}</p> : null}
      {hasQuery && state === "idle" && results.length === 0 ? (
        <p className="vault-state vault-state--inline">No matching files.</p>
      ) : null}
      {results.length > 0 ? (
        <div className="vault-search__results">
          {results.map((node) => {
            const isSelected = selectedPath === node.path;
            return (
              <button
                type="button"
                className={`vault-search__result${isSelected ? " vault-search__result--selected" : ""}`}
                aria-current={isSelected ? "page" : undefined}
                key={node.path}
                onClick={() => onSelectFile(node)}
              >
                <FileIcon extension={node.extension ?? ""} />
                <span className="vault-search__result-name">{node.name}</span>
                <span className="vault-search__result-path">{node.path}</span>
              </button>
            );
          })}
          {truncated ? (
            <p className="vault-state vault-state--inline">Search truncated. Refine the query for more specific results.</p>
          ) : null}
        </div>
      ) : null}
    </section>
  );
}

function WardTree({
  nodes,
  childrenByPath,
  truncatedByPath,
  expanded,
  loadingPaths,
  selectedPath,
  onToggle,
  onSelectFile,
  depth = 0,
}: {
  nodes: VaultNode[];
  childrenByPath: Record<string, VaultNode[]>;
  truncatedByPath: Record<string, boolean>;
  expanded: Set<string>;
  loadingPaths: Set<string>;
  selectedPath: string | null;
  onToggle: (node: VaultNode) => void;
  onSelectFile: (node: VaultNode) => void;
  depth?: number;
}) {
  return (
    <div className="vault-tree">
      {nodes.map((node) => {
        const isDirectory = node.kind === "directory";
        const isOpen = expanded.has(node.path);
        const isSelected = selectedPath === node.path;
        return (
          <div key={node.path || node.name}>
            <button
              type="button"
              aria-expanded={isDirectory ? isOpen : undefined}
              aria-current={isSelected ? "page" : undefined}
              className={`vault-tree-row${isSelected ? " vault-tree-row--selected" : ""}`}
              style={{ paddingLeft: `${8 + depth * 18}px` }}
              onClick={() => isDirectory ? onToggle(node) : onSelectFile(node)}
            >
              {isDirectory ? (
                isOpen ? <FolderOpen size={16} /> : <Folder size={16} />
              ) : (
                <FileIcon extension={node.extension ?? ""} />
              )}
              <span className="vault-tree-row__name">{node.name}</span>
            </button>
            {isDirectory && isOpen ? (
              <>
                {loadingPaths.has(node.path) ? (
                  <p className="vault-state vault-state--inline">Loading...</p>
                ) : null}
                <WardTree
                  nodes={childrenByPath[node.path] ?? []}
                  childrenByPath={childrenByPath}
                  truncatedByPath={truncatedByPath}
                  expanded={expanded}
                  loadingPaths={loadingPaths}
                  selectedPath={selectedPath}
                  onToggle={onToggle}
                  onSelectFile={onSelectFile}
                  depth={depth + 1}
                />
                {truncatedByPath[node.path] ? (
                  <p className="vault-state vault-state--inline">Directory truncated at 1,000 entries.</p>
                ) : null}
              </>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}
