"use client";

import dynamic from "next/dynamic";
import Skeleton from "@mui/material/Skeleton";

const DecisionTableEditor = dynamic(() => import("./DecisionTableEditorInner"), {
  ssr: false,
  loading: () => <Skeleton variant="rounded" height={560} />,
});

export default DecisionTableEditor;
