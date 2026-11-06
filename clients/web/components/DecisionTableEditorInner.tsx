"use client";

import { DecisionTable, type DecisionTableType } from "@gorules/jdm-editor";
import "@gorules/jdm-editor/dist/style.css";
import Box from "@mui/material/Box";

import { LOAN_FIELD_SCHEMA } from "@/lib/loan-fields";

/// Standalone GoRules decision-table editor — the "simple" authoring mode.
/// Self-contained (creates its own store); `mode="business"` swaps cell
/// inputs for guided pickers and `inputsSchema` powers the field catalog.
export default function DecisionTableEditorInner({
  value,
  onChange,
  disabled,
}: {
  value: DecisionTableType;
  onChange: (v: DecisionTableType) => void;
  disabled?: boolean;
}) {
  return (
    <Box
      sx={{
        border: 1,
        borderColor: "divider",
        borderRadius: 1,
        overflow: "hidden",
      }}
    >
      <DecisionTable
        tableHeight={560}
        value={value}
        onChange={onChange}
        disabled={disabled}
        mode="business"
        inputsSchema={LOAN_FIELD_SCHEMA}
        mountDialogsOnBody
      />
    </Box>
  );
}
