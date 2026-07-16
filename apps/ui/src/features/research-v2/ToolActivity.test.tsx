import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { ToolActivity } from "./ToolActivity";
import type { TimelineEntry } from "./types";

function toolCall(name: string, id = name): TimelineEntry {
  return {
    id,
    at: 1,
    kind: "tool_call",
    text: name,
    toolName: name,
  };
}

describe("<ToolActivity>", () => {
  it("retains a completed recall call without displaying its arguments or result", () => {
    render(<ToolActivity entries={[toolCall("recall"), toolCall("respond", "respond")]} />);

    expect(screen.getByTestId("turn-tool-activity")).toHaveTextContent("Tools used");
    expect(screen.getByTestId("turn-tool-activity")).toHaveTextContent("recall");
    expect(screen.getByTestId("turn-tool-activity")).toHaveTextContent("Running recall");
    expect(screen.queryByText("respond")).not.toBeInTheDocument();
  });

  it("renders nothing when the turn used no non-response tools", () => {
    const { container } = render(<ToolActivity entries={[toolCall("respond")]} />);
    expect(container).toBeEmptyDOMElement();
  });
});
