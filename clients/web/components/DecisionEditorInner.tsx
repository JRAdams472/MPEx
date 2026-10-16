"use client";

import { DecisionGraph, type DecisionGraphType } from "@gorules/jdm-editor";
import "@gorules/jdm-editor/dist/style.css";
import Box from "@mui/material/Box";

/// Embedded GoRules JDM editor — imported via next/dynamic with ssr:false
/// (editor internals are browser-only). Its stylesheet is scoped to its own
/// classes, so the app theme stays isolated from it.
export default function DecisionEditorInner({
  value,
  onChange,
  disabled,
}: {
  value: DecisionGraphType;
  onChange: (v: DecisionGraphType) => void;
  disabled?: boolean;
}) {
  return (
    <Box
      sx={{
        height: 560,
        border: 1,
        borderColor: "divider",
        borderRadius: 1,
        overflow: "hidden",
        "& .grl-dg": { height: "100%" },
      }}
    >
      <DecisionGraph value={value} onChange={onChange} disabled={disabled} />
    </Box>
  );
}
