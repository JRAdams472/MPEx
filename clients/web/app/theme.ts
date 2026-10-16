"use client";

import { createTheme } from "@mui/material/styles";

// GitHub-dark-inspired: near-black surfaces, laser-blue accent.
const theme = createTheme({
  palette: {
    mode: "dark",
    primary: { main: "#2f9cff" },
    secondary: { main: "#79c0ff" },
    success: { main: "#3fb950" },
    warning: { main: "#d29922" },
    error: { main: "#f85149" },
    background: { default: "#0d1117", paper: "#161b22" },
    text: { primary: "#e6edf3", secondary: "#9198a1" },
    divider: "#30363d",
  },
  components: {
    MuiCssBaseline: {
      styleOverrides: { body: { backgroundColor: "#0d1117" } },
    },
  },
});

export default theme;
