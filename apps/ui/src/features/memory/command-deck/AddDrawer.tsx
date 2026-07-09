import { useState } from "react";
import type { CreatableMemoryCategory } from "@/services/transport/types";

const CATEGORIES: CreatableMemoryCategory[] = [
  "pattern",
  "preference",
  "decision",
  "entity",
  "domain",
  "strategy",
  "skill",
  "user",
  "agent",
  "ward",
];

interface Props {
  initialCategory: CreatableMemoryCategory;
  wardId: string;
  onSave: (v: { category: CreatableMemoryCategory; content: string; ward_id: string }) => void;
  onClose: () => void;
}

export function AddDrawer({ initialCategory, wardId, onSave, onClose }: Props) {
  const [content, setContent] = useState("");
  const [category, setCategory] = useState<CreatableMemoryCategory>(initialCategory);

  return (
    <div role="dialog" aria-modal="true" aria-label="Add memory" className="add-drawer">
      <label className="add-drawer__label" htmlFor="add-drawer-category">
        <span>Category</span>
        <select
          id="add-drawer-category"
          value={category}
          onChange={(e) => setCategory(e.target.value as CreatableMemoryCategory)}
          className="add-drawer__select"
        >
          {CATEGORIES.map((c) => (
            <option key={c} value={c}>
              {c}
            </option>
          ))}
        </select>
      </label>
      <label className="add-drawer__label sr-only" htmlFor="add-drawer-content">
        Memory content
      </label>
      <textarea
        id="add-drawer-content"
        rows={4}
        value={content}
        onChange={(e) => setContent(e.target.value)}
        aria-label="Memory content"
        className="add-drawer__textarea"
        placeholder="Enter the memory content…"
      />
      <div className="add-drawer__actions">
        <button type="button" className="btn btn--ghost" onClick={onClose}>
          Cancel
        </button>
        <button
          type="button"
          className="btn btn--primary"
          disabled={!content.trim()}
          onClick={() => onSave({ category, content, ward_id: wardId })}
        >
          Save
        </button>
      </div>
    </div>
  );
}
