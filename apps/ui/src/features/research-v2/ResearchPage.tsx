// =============================================================================
// ResearchPage — top-level page component for the research-v2 feature.
//
// Vertical zones, top to bottom:
//   1. Header  — drawer toggle · title · ward chip + new + stop
//   2. Pill strip — StatusPill (centered)
//   3. Body    — scrollable column, with an optional ward vault rail
//   4. Artifact strip — live chips, hidden when state.artifacts is empty (R14d)
//   5. Composer — ChatInput pinned at the bottom
//
// Clicking a chip in the strip opens ArtifactSlideOut (shared with chat).
// =============================================================================

import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { Menu, PanelLeftOpen, Plus, Square } from "lucide-react";
import { toast } from "sonner";
import { ChatInput, type UploadedFile } from "../chat/ChatInput";
import { HeroInput } from "../chat/HeroInput";
import { useRecentSessions } from "../chat/mission-hooks";
import { isChatSession } from "@/services/session-kind";

type UploadedFileShim = UploadedFile;
import { ArtifactSlideOut } from "../chat/ArtifactSlideOut";
import { StatusPill } from "../shared/statusPill";
import { SessionTurnBlock } from "./SessionTurnBlock";
import { ArtifactStrip } from "./ArtifactStrip";
import { IntentInfoButton } from "./IntentInfoButton";
import { SessionsDrawer } from "./SessionsDrawer";
import { useResearchSession } from "./useResearchSession";
import { useSessionsList } from "./useSessionsList";
import { getTransport } from "@/services/transport";
import type { ResearchArtifactRef, ResearchSessionState } from "./types";
import type { Artifact } from "@/services/transport/types";
import { WardVaultExplorer } from "../vault/WardVaultExplorer";
import { VaultFileSlideOut } from "../vault/VaultFileSlideOut";
import { A2uiSurfaceRenderer } from "../surfaces/A2uiSurfaceRenderer";
import { useVaultFilePreview } from "../vault/useVaultFilePreview";
import type { WorkSurface } from "@/services/transport/types";
import "./research.css";

// --- Title derivation --------------------------------------------------------

const DEFAULT_RESEARCH_TITLE = "New research";
const TITLE_FIRST_MSG_MAX = 60;

/**
 * Derive a user-facing session title.
 * Priority: server-pushed title (session_title_changed event) → first user
 * message (truncated) → the "New research" placeholder. Simple prompts
 * ("what is 2 + 2") never trigger the backend title tool, so without the
 * message fallback the header would stay on the placeholder forever.
 */
function deriveTitle(state: ResearchSessionState): string {
  if (state.title && state.title.trim().length > 0) return state.title;
  // Multi-turn: the title is derived from the FIRST turn's user message.
  const firstUserMsg = state.turns[0]?.userMessage.content ?? "";
  const trimmed = firstUserMsg.trim();
  if (trimmed.length === 0) return DEFAULT_RESEARCH_TITLE;
  if (trimmed.length <= TITLE_FIRST_MSG_MAX) return trimmed;
  return trimmed.slice(0, TITLE_FIRST_MSG_MAX - 1) + "…";
}

// --- Sub-components ----------------------------------------------------------

interface ResearchHeaderProps {
  state: ResearchSessionState;
  onOpenDrawer(): void;
  onNew(): void;
  onStop(): void;
  /** Hide the "New research" button on the landing page since the hero
   *  already provides the new-session entry point. */
  showNewButton?: boolean;
}

function ResearchHeader({ state, onOpenDrawer, onNew, onStop, showNewButton = true }: ResearchHeaderProps) {
  return (
    <div className="research-page__header">
      <button
        type="button"
        className="btn btn--ghost btn--sm"
        onClick={onOpenDrawer}
        aria-label="Open sessions"
        title="Open sessions"
      >
        <Menu size={16} />
      </button>

      <div className="research-page__title" title={deriveTitle(state)}>
        {deriveTitle(state)}
        {state.sessionId && <IntentInfoButton sessionId={state.sessionId} />}
      </div>

      <div className="research-page__header-actions">
        {showNewButton && (
          <button type="button" className="btn btn--ghost btn--sm" onClick={onNew}>
            <Plus size={14} /> New research
          </button>
        )}
        {state.status === "running" && (
          <button
            type="button"
            className="btn btn--ghost btn--sm"
            onClick={onStop}
            title="Stop"
          >
            <Square size={14} />
          </button>
        )}
      </div>
    </div>
  );
}

function IntentLine({ state }: { state: ResearchSessionState }) {
  if (state.intentAnalyzing) {
    return <div className="research-page__intent-muted">analyzing intent…</div>;
  }
  if (state.intentClassification) {
    return (
      <div className="research-page__intent-classification">
        intent: <strong>{state.intentClassification}</strong>
        {state.wardName && (
          <>
            {" · ward: "}
            <strong>{state.wardName}</strong>
          </>
        )}
      </div>
    );
  }
  return null;
}

interface EmptyHeroProps {
  onSend: (message: string, attachments: UploadedFileShim[]) => void;
}

// Re-use the chat HeroInput visual for the research-v2 landing page. The
// recent-session card click routes via React Router to /research-v2/:id
// (instead of the chat mission-control switcher).
function EmptyHero({ onSend }: EmptyHeroProps) {
  const navigate = useNavigate();
  // Exclude chat-mode sessions from the research landing's recent cards
  // so chat-v2 turns don't present themselves as research starters.
  // Classification lives in services/session-kind — see that module for
  // the mode/prefix fallback logic.
  const { sessions: recentSessions } = useRecentSessions({ exclude: isChatSession });
  return (
    <HeroInput
      onSend={onSend}
      recentSessions={recentSessions}
      onSelectSession={(_sessionId, conversationId) => {
        navigate(`/research/${conversationId}`);
      }}
    />
  );
}

interface MainColumnProps {
  state: ResearchSessionState;
  surfaces: WorkSurface[];
  onSend: (message: string, attachments: UploadedFileShim[]) => void;
}

function MainColumn({ state, surfaces, onSend }: MainColumnProps) {
  const hasContent = state.turns.length > 0 || state.sessionId !== null;

  if (!hasContent) return <EmptyHero onSend={onSend} />;

  return (
    <>
      <IntentLine state={state} />
      {state.turns.map((turn) => (
        <SessionTurnBlock key={turn.id} turn={turn} />
      ))}
      {(surfaces ?? []).map(surface => <A2uiSurfaceRenderer key={surface.surface_id} surface={surface} />)}
    </>
  );
}

// --- Page --------------------------------------------------------------------

export function ResearchPage() {
  const { state, pillState, surfaces, wardVaultRevision, sendMessage, stopAgent, startNewResearch, getFullArtifact } =
    useResearchSession();
  const { sessions, refresh: refreshSessions, deleteSession } = useSessionsList({
    onAfterDelete: (deletedId) => {
      if (state.sessionId === deletedId) startNewResearch();
    },
  });
  const navigate = useNavigate();
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [viewingArtifact, setViewingArtifact] = useState<Artifact | null>(null);
  const [vaultCollapsed, setVaultCollapsed] = useState(false);
  const researchWard = state.wardId && state.wardName
    ? { id: state.wardId, name: state.wardName }
    : null;
  const {
    selectedFile: selectedVaultFile,
    selectFile: selectVaultFile,
    clearSelectedFile: clearSelectedVaultFile,
  } = useVaultFilePreview(researchWard?.id ?? null);

  // Reflect the session title in the browser tab + refresh the drawer list
  // when the server pushes a new title (so the sidebar row renames live).
  const derivedTitle = deriveTitle(state);
  useEffect(() => {
    document.title = state.sessionId
      ? `${derivedTitle} · z-Bot`
      : "z-Bot - Web Dashboard";
  }, [derivedTitle, state.sessionId]);
  useEffect(() => {
    if (state.title && state.sessionId) void refreshSessions();
  }, [state.title, state.sessionId, refreshSessions]);

  // R14d — Decision B: state.artifacts holds the lightweight refs (keeps
  // reducer tests stable); the hook caches the full Artifact records from
  // the poll and resolves by id here. Fallback path fetches once if the
  // user clicks before the first poll completes (edge case).
  const handleOpenArtifact = useCallback(
    async (ref: ResearchArtifactRef) => {
      const cached = getFullArtifact(ref.id);
      if (cached) {
        setViewingArtifact(cached);
        return;
      }
      if (!state.sessionId) return;
      const transport = await getTransport();
      const result = await transport.listSessionArtifacts(state.sessionId);
      if (!result.success || !result.data) {
        toast.error(`Failed to open artifact: ${!result.success ? result.error : "not found"}`);
        return;
      }
      const match = result.data.find((a) => a.id === ref.id);
      if (match) setViewingArtifact(match);
      else toast.error("Artifact not found");
    },
    [getFullArtifact, state.sessionId]
  );

  const handleSelect = (id: string) => {
    setDrawerOpen(false);
    navigate(`/research/${id}`);
  };

  const handleNew = () => {
    setDrawerOpen(false);
    startNewResearch();
    void refreshSessions();
  };

  const composerDisabled = state.status === "running";
  // Landing state: no user message, no agent turns, no bound session. Hero
  // takes over the column; the bottom composer + the header's "New
  // research" button are hidden so the landing experience is uncluttered.
  const isLanding = state.turns.length === 0 && state.sessionId === null;

  return (
    <div className={`research-page${researchWard ? " research-page--with-vault" : ""}${vaultCollapsed ? " research-page--vault-collapsed" : ""}`}>
      <ResearchHeader
        state={state}
        onOpenDrawer={() => setDrawerOpen(true)}
        onNew={handleNew}
        onStop={stopAgent}
        showNewButton={!isLanding}
      />

      {!isLanding && (
        <div className="research-page__pill-strip">
          <StatusPill state={pillState} />
        </div>
      )}

      <SessionsDrawer
        open={drawerOpen}
        onClose={() => setDrawerOpen(false)}
        sessions={sessions}
        currentId={state.sessionId}
        onSelect={handleSelect}
        onNew={handleNew}
        onDelete={deleteSession}
      />

      <div className={`research-page__body${researchWard ? " research-page__body--with-vault" : ""}${vaultCollapsed ? " research-page__body--vault-collapsed" : ""}`}>
        {researchWard ? (
          <>
            <div className={`research-page__vault-shell${vaultCollapsed ? " research-page__vault-shell--collapsed" : ""}`}>
              <WardVaultExplorer
                ward={researchWard}
                refreshKey={wardVaultRevision}
                selectedPath={selectedVaultFile?.node.path ?? null}
                onSelectFile={(node) => void selectVaultFile(node)}
                onCollapse={() => setVaultCollapsed(true)}
                ariaLabel="Research ward vault explorer"
              />
            </div>
            {vaultCollapsed ? (
              <button
                type="button"
                className="research-page__vault-toggle"
                onClick={() => setVaultCollapsed(false)}
                aria-label={`Expand ward vault explorer for ${researchWard.name}`}
              >
                <PanelLeftOpen size={16} />
                <span>{researchWard.name}</span>
              </button>
            ) : null}
          </>
        ) : null}
        <div className="research-page__column">
          <MainColumn state={state} surfaces={surfaces} onSend={sendMessage} />
        </div>
      </div>

      {!isLanding && (
        <>
          <ArtifactStrip artifacts={state.artifacts} onOpen={handleOpenArtifact} />
          <div className="research-page__composer">
            <ChatInput onSend={sendMessage} disabled={composerDisabled} />
          </div>
        </>
      )}

      {viewingArtifact && (
        <ArtifactSlideOut
          artifact={viewingArtifact}
          onClose={() => setViewingArtifact(null)}
        />
      )}

      {selectedVaultFile && (
        <VaultFileSlideOut
          selected={selectedVaultFile}
          onClose={clearSelectedVaultFile}
        />
      )}
    </div>
  );
}
