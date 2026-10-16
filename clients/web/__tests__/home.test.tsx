import "@testing-library/jest-dom";
import { render, screen } from "@testing-library/react";

import Home from "../app/page";

describe("Home", () => {
  it("renders the MPEx heading", () => {
    render(<Home />);
    expect(
      screen.getByRole("heading", { name: /mpex/i }),
    ).toBeInTheDocument();
  });
});
