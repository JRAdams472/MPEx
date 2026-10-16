import "@testing-library/jest-dom";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import Home from "../app/page";

function renderWithQuery(ui: React.ReactElement) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(<QueryClientProvider client={client}>{ui}</QueryClientProvider>);
}

describe("Home", () => {
  it("renders the dashboard with section cards", () => {
    renderWithQuery(<Home />);
    expect(
      screen.getByRole("heading", { name: /dashboard/i }),
    ).toBeInTheDocument();
    for (const label of ["Products", "Decisions", "Expressions", "Publications"]) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
  });
});
