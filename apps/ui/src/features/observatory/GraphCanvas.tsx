// ============================================================================
// GRAPH CANVAS — D3-force knowledge graph rendered in SVG
// ============================================================================

import { useRef, useEffect, useCallback } from "react";
import {
  forceSimulation,
  forceLink,
  forceManyBody,
  forceCenter,
  forceCollide,
  type SimulationNodeDatum,
  type SimulationLinkDatum,
} from "d3-force";
import { select } from "d3-selection";
import { zoom as d3Zoom, zoomIdentity, type ZoomBehavior } from "d3-zoom";
import type { GraphEntity, GraphRelationship } from "@/services/transport/types";
import { ZoomIn, ZoomOut, RotateCcw } from "lucide-react";

// ============================================================================
// Types
// ============================================================================

interface GraphCanvasProps {
  entities: GraphEntity[];
  relationships: GraphRelationship[];
  selectedEntityId?: string;
  highlightTerm?: string;
  onEntitySelect: (entity: GraphEntity) => void;
}

/** D3 simulation node wrapping a GraphEntity. */
interface SimNode extends SimulationNodeDatum {
  id: string;
  entity: GraphEntity;
}

/** D3 simulation link wrapping a GraphRelationship. */
interface SimLink extends SimulationLinkDatum<SimNode> {
  relationship: GraphRelationship;
}

// ============================================================================
// Constants
// ============================================================================

const ENTITY_TYPE_COLORS: Record<string, string> = {
  person: "#6366f1",
  concept: "#f59e0b",
  agent: "#10b981",
  tool: "#10b981",
  project: "#ef4444",
  strategy: "#8b5cf6",
};

const ENTITY_TYPE_CLASS: Record<string, string> = {
  person: "graph-node--person",
  concept: "graph-node--concept",
  agent: "graph-node--agent",
  tool: "graph-node--tool",
  project: "graph-node--project",
  strategy: "graph-node--strategy",
};

function nodeRadius(mention_count: number): number {
  return 6 + Math.min(mention_count, 20);
}

function matchesHighlight(entity: GraphEntity, term: string): boolean {
  if (!term) return true;
  const lower = term.toLowerCase();
  return (
    entity.name.toLowerCase().includes(lower) ||
    entity.entity_type.toLowerCase().includes(lower)
  );
}

function linkTouchesEntity(link: SimLink, entityId: string): boolean {
  const source = link.source as SimNode | string;
  const target = link.target as SimNode | string;
  const sourceId = typeof source === "string" ? source : source.id;
  const targetId = typeof target === "string" ? target : target.id;
  return sourceId === entityId || targetId === entityId;
}

// ============================================================================
// Component
// ============================================================================

export function GraphCanvas({
  entities,
  relationships,
  selectedEntityId,
  highlightTerm,
  onEntitySelect,
}: GraphCanvasProps) {
  const svgRef = useRef<SVGSVGElement>(null);
  const gRef = useRef<SVGGElement | null>(null);
  const simulationRef = useRef<ReturnType<typeof forceSimulation<SimNode>> | null>(null);
  const zoomBehaviorRef = useRef<ZoomBehavior<SVGSVGElement, unknown> | null>(null);

  // Build nodes + links from entity/relationship data
  const buildGraph = useCallback(() => {
    const nodes: SimNode[] = entities.map((e) => ({
      id: e.id,
      entity: e,
    }));

    const nodeIdSet = new Set(entities.map((e) => e.id));

    const links: SimLink[] = relationships
      .filter(
        (r) =>
          nodeIdSet.has(r.source_entity_id) && nodeIdSet.has(r.target_entity_id)
      )
      .map((r) => ({
        source: r.source_entity_id,
        target: r.target_entity_id,
        relationship: r,
      }));

    return { nodes, links };
  }, [entities, relationships]);

  // D3 rendering effect
  useEffect(() => {
    const svg = svgRef.current;
    if (!svg) return;
    if (entities.length === 0) return;

    const { nodes, links } = buildGraph();

    const svgSel = select(svg);
    const width = svg.clientWidth || 800;
    const height = svg.clientHeight || 600;

    // Clear previous content
    svgSel.selectAll("g.graph-root").remove();

    // Root group for zoom/pan
    const g = svgSel.append("g").attr("class", "graph-root");
    gRef.current = g.node();

    // Zoom behavior
    const zoomBehavior = d3Zoom<SVGSVGElement, unknown>()
      .scaleExtent([0.2, 5])
      .on("zoom", (event) => {
        g.attr("transform", event.transform);
      });

    svgSel.call(zoomBehavior);
    zoomBehaviorRef.current = zoomBehavior;

    // Edges
    const linkSel = g
      .selectAll<SVGLineElement, SimLink>("line.graph-edge")
      .data(links)
      .join("line")
      .attr("class", "graph-edge")
      .style("--graph-edge-strength", (d) =>
        String(Math.max(0.15, Math.min(0.6, (d.relationship.mention_count || 1) * 0.15)))
      );

    // Node groups
    const nodeSel = g
      .selectAll<SVGGElement, SimNode>("g.graph-node")
      .data(nodes, (d) => d.id)
      .join("g")
      .attr("class", (d) => {
        const base = "graph-node";
        const typeClass = ENTITY_TYPE_CLASS[d.entity.entity_type.toLowerCase()] || "";
        return `${base} ${typeClass}`.trim();
      })
      .on("click", (_event, d) => {
        onEntitySelect(d.entity);
      });

    // Circles
    nodeSel
      .append("circle")
      .attr("r", (d) => nodeRadius(d.entity.mention_count))
      .attr("class", (d) => {
        const typeClass = ENTITY_TYPE_CLASS[d.entity.entity_type.toLowerCase()] || "";
        return typeClass;
      });

    // Labels
    nodeSel
      .append("text")
      .attr("class", "graph-label")
      .attr("dy", (d) => nodeRadius(d.entity.mention_count) + 12)
      .text((d) => d.entity.name);

    // Force simulation
    const simulation = forceSimulation<SimNode>(nodes)
      .force(
        "link",
        forceLink<SimNode, SimLink>(links)
          .id((d) => d.id)
          .distance(80)
      )
      .force("charge", forceManyBody().strength(-200))
      .force("center", forceCenter(width / 2, height / 2))
      .force(
        "collide",
        forceCollide<SimNode>().radius(
          (d) => nodeRadius(d.entity.mention_count) + 4
        )
      )
      .on("tick", () => {
        linkSel
          .attr("x1", (d) => ((d.source as unknown as SimNode).x ?? 0))
          .attr("y1", (d) => ((d.source as unknown as SimNode).y ?? 0))
          .attr("x2", (d) => ((d.target as unknown as SimNode).x ?? 0))
          .attr("y2", (d) => ((d.target as unknown as SimNode).y ?? 0));

        nodeSel.attr("transform", (d) => `translate(${d.x ?? 0},${d.y ?? 0})`);
      });

    simulationRef.current = simulation;

    // Drag behavior via mousedown/mousemove/mouseup on each node group
    nodeSel.on("mousedown.drag", function (event: MouseEvent, d: SimNode) {
      event.stopPropagation();
      simulation.alphaTarget(0.3).restart();
      d.fx = d.x;
      d.fy = d.y;

      const onMove = (e: MouseEvent) => {
        // Parse transform from root group to account for zoom/pan
        const transformStr = gRef.current?.getAttribute("transform") || "";
        const scaleMatch = transformStr.match(/scale\(([\d.e+-]+)\)/);
        const translateMatch = transformStr.match(/translate\(([\d.e+-]+),\s*([\d.e+-]+)\)/);
        const scale = scaleMatch ? Number.parseFloat(scaleMatch[1]) : 1;
        const tx = translateMatch ? Number.parseFloat(translateMatch[1]) : 0;
        const ty = translateMatch ? Number.parseFloat(translateMatch[2]) : 0;
        const rect = svg.getBoundingClientRect();
        d.fx = (e.clientX - rect.left - tx) / scale;
        d.fy = (e.clientY - rect.top - ty) / scale;
      };

      const onUp = () => {
        simulation.alphaTarget(0);
        d.fx = null;
        d.fy = null;
        window.removeEventListener("mousemove", onMove);
        window.removeEventListener("mouseup", onUp);
      };

      window.addEventListener("mousemove", onMove);
      window.addEventListener("mouseup", onUp);
    });

    return () => {
      simulation.stop();
      svgSel.selectAll("g.graph-root").remove();
      svgSel.on(".zoom", null);
    };
  }, [entities, relationships, buildGraph, onEntitySelect]);

  // Update selection + highlight classes reactively
  useEffect(() => {
    const g = gRef.current;
    if (!g) return;

    const gSel = select(g);

    // Selection highlight on circles
    gSel
      .selectAll<SVGCircleElement, SimNode>("g.graph-node circle")
      .classed("graph-node--selected", (_d, i, nodes) => {
        const parent = nodes[i].parentNode as Element | null;
        if (!parent) return false;
        const datum = select<Element, SimNode>(parent).datum();
        return datum?.entity.id === selectedEntityId;
      });

    // Highlight / dimming on node groups
    const hasTerm = !!highlightTerm && highlightTerm.length > 0;

    gSel
      .selectAll<SVGGElement, SimNode>("g.graph-node")
      .classed("graph-node--dimmed", (_d, i, nodes) => {
        if (!hasTerm) return false;
        const datum = select<SVGGElement, SimNode>(nodes[i]).datum();
        return !matchesHighlight(datum.entity, highlightTerm!);
      });

    gSel
      .selectAll<SVGTextElement, SimNode>("text.graph-label")
      .classed("graph-label--dimmed", (_d, i, nodes) => {
        if (!hasTerm) return false;
        const parent = nodes[i].parentNode as Element | null;
        if (!parent) return false;
        const datum = select<Element, SimNode>(parent).datum();
        return !matchesHighlight(datum.entity, highlightTerm!);
      });

    gSel
      .selectAll<SVGLineElement, SimLink>("line.graph-edge")
      .classed("graph-edge--related", (d) =>
        selectedEntityId ? linkTouchesEntity(d, selectedEntityId) : false
      )
      .classed("graph-edge--dimmed", (d) => {
        if (!hasTerm) return false;
        const source = d.source as SimNode | string;
        const target = d.target as SimNode | string;
        const sourceEntity = typeof source === "string" ? undefined : source.entity;
        const targetEntity = typeof target === "string" ? undefined : target.entity;
        return !(
          (sourceEntity && matchesHighlight(sourceEntity, highlightTerm!)) ||
          (targetEntity && matchesHighlight(targetEntity, highlightTerm!))
        );
      });
  }, [selectedEntityId, highlightTerm]);

  // Zoom controls — no d3-transition, use direct transform
  const handleZoom = useCallback((direction: "in" | "out" | "reset") => {
    const svg = svgRef.current;
    const zoomBehavior = zoomBehaviorRef.current;
    if (!svg || !zoomBehavior) return;

    const svgSel = select<SVGSVGElement, unknown>(svg);

    if (direction === "reset") {
      zoomBehavior.transform(svgSel, zoomIdentity);
    } else {
      const factor = direction === "in" ? 1.4 : 0.7;
      zoomBehavior.scaleBy(svgSel, factor);
    }
  }, []);

  // Empty state
  if (entities.length === 0) {
    return (
      <div className="observatory__canvas">
        <div
          className="empty-state"
          style={{
            height: "100%",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
          }}
        >
          <div style={{ textAlign: "center" }}>
            <p className="empty-state__title">No graph data</p>
            <p className="empty-state__description">
              Entities and relationships will appear here after conversations
              are distilled.
            </p>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="observatory__canvas observatory__canvas--graph">
      <svg ref={svgRef} aria-label="Interactive knowledge graph">
        <defs>
          <radialGradient id="observatory-graph-glow" cx="50%" cy="45%" r="70%">
            <stop offset="0%" className="graph-atmosphere__core" />
            <stop offset="100%" className="graph-atmosphere__edge" />
          </radialGradient>
          <pattern id="observatory-graph-grid" width="32" height="32" patternUnits="userSpaceOnUse">
            <path d="M 32 0 L 0 0 0 32" className="graph-atmosphere__grid" />
          </pattern>
        </defs>
        <g className="graph-atmosphere" aria-hidden="true">
          <rect width="100%" height="100%" fill="url(#observatory-graph-glow)" />
          <rect width="100%" height="100%" fill="url(#observatory-graph-grid)" />
        </g>
      </svg>

      <div className="observatory__graph-context">
        <span className="observatory__graph-kicker">Knowledge network</span>
        <span>{entities.length} entities · {relationships.length} relationships</span>
      </div>

      {/* Legend */}
      <div className="observatory__legend">
        {Object.entries(ENTITY_TYPE_COLORS)
          .filter(([key]) => key !== "tool") // tool shares color with agent
          .map(([type, color]) => (
            <span key={type} className="observatory__legend-item">
              <span
                className="observatory__legend-dot"
                style={{ backgroundColor: color }}
              />
              {type}
            </span>
          ))}
      </div>

      {/* Zoom controls */}
      <div className="observatory__zoom-controls">
        <button
          className="observatory__zoom-btn"
          onClick={() => handleZoom("in")}
          title="Zoom in"
        >
          <ZoomIn style={{ width: 14, height: 14 }} />
        </button>
        <button
          className="observatory__zoom-btn"
          onClick={() => handleZoom("out")}
          title="Zoom out"
        >
          <ZoomOut style={{ width: 14, height: 14 }} />
        </button>
        <button
          className="observatory__zoom-btn"
          onClick={() => handleZoom("reset")}
          title="Reset view"
        >
          <RotateCcw style={{ width: 14, height: 14 }} />
        </button>
      </div>
    </div>
  );
}
