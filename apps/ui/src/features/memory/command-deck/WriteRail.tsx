import { useState } from "react";
import type { CreatableMemoryCategory } from "@/services/transport/types";
import { AddDrawer } from "./AddDrawer";

type WriteChoice = { label: string; category: CreatableMemoryCategory; key: string };
const CHOICES: WriteChoice[] = [
  { label: "+ Fact", category: "pattern", key: "F" },
  { label: "+ Preference", category: "preference", key: "P" },
  { label: "+ Decision", category: "decision", key: "D" },
];

interface Props {
  wardId: string;
  counts: { facts: number; wiki: number; procedures: number; episodes: number };
  onSave: (v: { category: CreatableMemoryCategory; content: string; ward_id: string }) => void;
}

export function WriteRail({ wardId, counts, onSave }: Props) {
  const [open, setOpen] = useState<CreatableMemoryCategory | null>(null);

  return (
    <aside className="memory-write">
      <div className="memory-write__title">WRITE</div>
      {CHOICES.map((c) => (
        <button
          key={c.label}
          type="button"
          className="memory-write__btn"
          onClick={() => setOpen(c.category)}
        >
          <span>{c.label}</span>
          <kbd>{c.key}</kbd>
        </button>
      ))}

      <div className="memory-write__stats">
        <div className="memory-write__ward">{wardId || "—"}</div>
        <div>facts {counts.facts}</div>
        <div>wiki {counts.wiki}</div>
        <div>procedures {counts.procedures}</div>
        <div>episodes {counts.episodes}</div>
      </div>

      {open && (
        <AddDrawer
          initialCategory={open}
          wardId={wardId}
          onClose={() => setOpen(null)}
          onSave={(v) => {
            onSave(v);
            setOpen(null);
          }}
        />
      )}
    </aside>
  );
}
