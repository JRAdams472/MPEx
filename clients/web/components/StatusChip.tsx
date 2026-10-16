import Chip from "@mui/material/Chip";

/// Artifact revision state: drafts are mutable working copies, published rows
/// are immutable and versioned.
export default function StatusChip({
  status,
  hasPublished,
}: {
  status: "draft" | "published";
  hasPublished?: boolean;
}) {
  if (status === "published") {
    return <Chip label="published" size="small" color="success" variant="outlined" />;
  }
  return (
    <Chip
      label={hasPublished ? "draft (edited)" : "draft"}
      size="small"
      color="warning"
      variant="outlined"
    />
  );
}
