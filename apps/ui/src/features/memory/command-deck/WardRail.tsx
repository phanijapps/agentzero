import type { CSSProperties } from "react";

interface Ward {
  id: string;
  count: number;
}

/**
 * Hash a ward id to a stable hue in [0, 360). Deterministic per string so the
 * same ward always gets the same color across sessions. Uses `codePointAt` to
 * respect the security-patterns rule (prefer codePointAt over charCodeAt).
 */
function wardHue(id: string): number {
  let h = 0;
  for (let i = 0; i < id.length; i++) {
    const cp = id.codePointAt(i) ?? 0;
    h = (h * 31 + cp) % 360;
  }
  return h;
}

interface Props {
  wards: Ward[];
  activeId: string;
  onSelect: (id: string) => void;
}

interface WardButtonProps {
  ward: Ward;
  activeId: string;
  onSelect: (id: string) => void;
  isGlobal?: boolean;
}

function WardButton({ ward, activeId, onSelect, isGlobal }: WardButtonProps) {
  const isActive = ward.id === activeId;
  const className = `memory-ward ${isActive ? "is-active" : ""}`.trim();
  const dotClass = isGlobal
    ? "memory-ward__dot memory-ward__dot--global"
    : "memory-ward__dot";
  const dotStyle: CSSProperties | undefined = isGlobal
    ? undefined
    : ({ "--ward-hue": `${wardHue(ward.id)}` } as CSSProperties);
  return (
    <button
      type="button"
      className={className}
      aria-current={isActive ? "true" : undefined}
      onClick={() => onSelect(ward.id)}
    >
      <span className={dotClass} style={dotStyle} aria-hidden="true" />
      <span className="memory-ward__name">{ward.id}</span>
      <span className="memory-ward__badge">{ward.count}</span>
    </button>
  );
}

export function WardRail({ wards, activeId, onSelect }: Props) {
  const regular = wards.filter((w) => !w.id.startsWith("__"));
  const global = wards.filter((w) => w.id.startsWith("__"));
  return (
    <nav className="memory-wards" aria-label="Memory scope">
      <header className="memory-wards__head">
        <div>
          <h2>Scope radar</h2>
          <p>Where to recall from</p>
        </div>
      </header>
      <div className="memory-wards__title">
        <span>WARDS</span>
        <span>{regular.length}</span>
      </div>
      <ul>
        {regular.map((w) => (
          <li key={w.id}>
            <WardButton ward={w} activeId={activeId} onSelect={onSelect} />
          </li>
        ))}
      </ul>
      {global.length > 0 && (
        <>
          <div className="memory-wards__title">
            <span>GLOBAL</span>
            <span>∞</span>
          </div>
          <ul>
            {global.map((w) => (
              <li key={w.id}>
                <WardButton ward={w} activeId={activeId} onSelect={onSelect} isGlobal />
              </li>
            ))}
          </ul>
        </>
      )}
    </nav>
  );
}
