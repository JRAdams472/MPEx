"use client";

import dynamic from "next/dynamic";
import Skeleton from "@mui/material/Skeleton";

const DecisionEditor = dynamic(() => import("./DecisionEditorInner"), {
  ssr: false,
  loading: () => <Skeleton variant="rounded" height={560} />,
});

export default DecisionEditor;
