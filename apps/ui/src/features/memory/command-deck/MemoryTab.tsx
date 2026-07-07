import { useEffect, useState } from "react";
import { SearchBar, type SearchMode } from "./SearchBar";
import { ScopeChips, type ContentType } from "./ScopeChips";
import { WardRail } from "./WardRail";
import { ContentDeck } from "./ContentDeck";
import { WriteRail } from "./WriteRail";
import { SearchResults } from "./SearchResults";
import { useWards, useWardContent, useHybridSearch, useTimewarp } from "./hooks";
import { getTransport } from "@/services/transport";
import type { CreatableMemoryCategory } from "@/services/transport/types";
import { BeliefsList } from "./beliefs/BeliefsList";
import { ContradictionList } from "./beliefs/ContradictionList";

interface Props {
  agentId: string;
}

/**
 * Top-level sub-tab. Defaults to "facts" so the existing flow is
 * preserved when callers don't know about beliefs yet.
 */
type MemorySubTab = "facts" | "beliefs" | "contradictions";

const SUB_TAB_LABELS: Record<MemorySubTab, string> = {
  facts: "Facts",
  beliefs: "Beliefs",
  contradictions: "Contradictions",
};

interface SaveInput {
  category: CreatableMemoryCategory;
  content: string;
  ward_id: string;
}

/** Build a stable-ish key from the first words of the content. */
function buildFactKey(content: string): string {
  const slug = content
    .toLowerCase()
    .replace(/[^a-z0-9\s-]/g, "")
    .trim()
    .split(/\s+/)
    .slice(0, 6)
    .join("-");
  return slug || `fact-${Date.now()}`;
}

export function MemoryTab({ agentId }: Props) {
  const wards = useWards();
  const [activeId, setActiveId] = useState<string>("");
  const [subTab, setSubTab] = useState<MemorySubTab>("facts");

  // Auto-select the first ward once wards load.
  useEffect(() => {
    if (!activeId && wards.length > 0) setActiveId(wards[0].id);
  }, [wards, activeId]);

  const { data, refresh } = useWardContent(activeId || null);

  const [query, setQuery] = useState("");
  const [mode, setMode] = useState<SearchMode>("hybrid");
  const [types, setTypes] = useState<ContentType[]>(["facts", "wiki"]);
  const { days, setDays } = useTimewarp();

  const search = useHybridSearch(query, {
    mode,
    types,
    ward_ids: activeId ? [activeId] : [],
  });
  const searching = query.trim().length > 0;

  async function saveFact(v: SaveInput) {
    const transport = await getTransport();
    await transport.createMemory(agentId, {
      category: v.category,
      key: buildFactKey(v.content),
      content: v.content,
      ward_id: v.ward_id,
    });
    await refresh();
  }

  async function deleteFact(factId: string) {
    const transport = await getTransport();
    const result = await transport.deleteMemory(agentId, factId);
    if (result.success) await refresh();
    else if (typeof window !== "undefined") {
      window.alert(`Failed to delete memory: ${result.error ?? "unknown error"}`);
    }
  }

  return (
    <div className="memory-tab-deck">
      <div className="memory-tab-deck__top">
        {subTab === "facts" ? (
          <>
            <SearchBar
              onChange={(v) => {
                setQuery(v.query);
                setMode(v.mode);
              }}
            />
            <div className="memory-tab-deck__row">
              <ScopeChips types={types} onChange={(v) => setTypes(v.types)} />
              <TimewarpSlider days={days} onChange={setDays} />
            </div>
          </>
        ) : null}
        <SubTabBar active={subTab} onChange={setSubTab} />
      </div>
      <div className="memory-tab-deck__grid">
        <WardRail wards={wards} activeId={activeId} onSelect={setActiveId} />
        <CenterPanel
          subTab={subTab}
          agentId={agentId}
          activeId={activeId}
          searching={searching}
          query={query}
          mode={mode}
          searchData={search.data}
          searchLoading={search.loading}
          data={data}
          days={days}
          onDeleteFact={deleteFact}
        />
        {subTab === "facts" ? (
          <WriteRail
            wardId={activeId}
            counts={
              data?.counts ?? { facts: 0, wiki: 0, procedures: 0, episodes: 0 }
            }
            onSave={(v) => {
              void saveFact(v);
            }}
          />
        ) : (
          <div className="memory-tab-deck__write-spacer" />
        )}
      </div>
    </div>
  );
}

interface SubTabBarProps {
  active: MemorySubTab;
  onChange: (t: MemorySubTab) => void;
}

function SubTabBar({ active, onChange }: SubTabBarProps) {
  const tabs: MemorySubTab[] = ["facts", "beliefs", "contradictions"];
  return (
    <div className="tab-bar memory-tab-deck__subtabs" role="tablist" aria-label="Memory sub-tabs">
      {tabs.map((t) => (
        <button
          key={t}
          type="button"
          role="tab"
          aria-selected={active === t}
          className={`tab-bar__tab${active === t ? " tab-bar__tab--active" : ""}`}
          onClick={() => onChange(t)}
        >
          {SUB_TAB_LABELS[t]}
        </button>
      ))}
    </div>
  );
}

interface CenterPanelProps {
  subTab: MemorySubTab;
  agentId: string;
  activeId: string;
  searching: boolean;
  query: string;
  mode: SearchMode;
  searchData: ReturnType<typeof useHybridSearch>["data"];
  searchLoading: boolean;
  data: ReturnType<typeof useWardContent>["data"];
  days: number;
  onDeleteFact: (id: string) => Promise<void>;
}

function CenterPanel(p: CenterPanelProps) {
  if (p.subTab === "beliefs") {
    return (
      <div className="memory-deck">
        <BeliefsList agentId={p.agentId} partitionId={p.activeId || null} />
      </div>
    );
  }
  if (p.subTab === "contradictions") {
    return (
      <div className="memory-deck">
        <ContradictionList agentId={p.agentId} partitionId={p.activeId || null} />
      </div>
    );
  }
  // Facts sub-tab — preserves the existing search-or-content behavior.
  if (p.searching) {
    return (
      <div className="memory-deck">
        <header className="memory-deck__head">
          <div className="memory-deck__crumb">
            ⌕ Search results {p.activeId ? `in ${p.activeId}` : "across all wards"}
          </div>
          <div className="memory-deck__summary">
            Query: <strong>{p.query}</strong> · mode: {p.mode}
          </div>
        </header>
        <div className="memory-deck__body">
          <SearchResults
            data={p.searchData}
            loading={p.searchLoading}
            onDeleteFact={p.onDeleteFact}
          />
        </div>
      </div>
    );
  }
  return (
    <ContentDeck
      data={p.data}
      timewarpDays={p.days}
      onDeleteFact={p.onDeleteFact}
    />
  );
}

function TimewarpSlider({ days, onChange }: { days: number; onChange: (d: number) => void }) {
  return (
    <label className="memory-timewarp" aria-label="Time range">
      <span className="memory-timewarp__tick">NOW</span>
      <input
        type="range"
        min={0}
        max={30}
        step={1}
        value={days}
        onChange={(e) => onChange(Number.parseInt(e.target.value, 10))}
        className="memory-timewarp__slider"
      />
      <span className="memory-timewarp__tick">{days === 0 ? "0d" : `${days}d`}</span>
    </label>
  );
}
