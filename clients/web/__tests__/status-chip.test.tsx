import "@testing-library/jest-dom";
import { render, screen } from "@testing-library/react";

import StatusChip from "../components/StatusChip";

describe("StatusChip", () => {
  it("marks published artifacts", () => {
    render(<StatusChip status="published" />);
    expect(screen.getByText("published")).toBeInTheDocument();
  });

  it("distinguishes fresh drafts from edited ones", () => {
    render(<StatusChip status="draft" hasPublished />);
    expect(screen.getByText("draft (edited)")).toBeInTheDocument();
  });

  it("plain draft when nothing is published", () => {
    render(<StatusChip status="draft" hasPublished={false} />);
    expect(screen.getByText("draft")).toBeInTheDocument();
  });
});
