"use client";

import Box from "@mui/material/Box";
import AppBar from "@mui/material/AppBar";
import Toolbar from "@mui/material/Toolbar";
import Typography from "@mui/material/Typography";
import Drawer from "@mui/material/Drawer";
import List from "@mui/material/List";
import ListItemButton from "@mui/material/ListItemButton";
import ListItemIcon from "@mui/material/ListItemIcon";
import ListItemText from "@mui/material/ListItemText";
import Divider from "@mui/material/Divider";
import Button from "@mui/material/Button";
import DashboardIcon from "@mui/icons-material/Dashboard";
import InventoryIcon from "@mui/icons-material/Inventory2";
import AccountTreeIcon from "@mui/icons-material/AccountTree";
import FunctionsIcon from "@mui/icons-material/Functions";
import GridOnIcon from "@mui/icons-material/GridOn";
import TableChartIcon from "@mui/icons-material/TableChart";
import ScienceIcon from "@mui/icons-material/Science";
import AdminPanelSettingsIcon from "@mui/icons-material/AdminPanelSettings";
import FactCheckIcon from "@mui/icons-material/FactCheck";
import PublishIcon from "@mui/icons-material/Publish";
import HistoryIcon from "@mui/icons-material/History";
import LogoutIcon from "@mui/icons-material/Logout";
import Chip from "@mui/material/Chip";
import IconButton from "@mui/material/IconButton";
import Tooltip from "@mui/material/Tooltip";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { useState } from "react";

import { useMe } from "@/lib/me";
import PublishDialog from "./PublishDialog";

const DRAWER_WIDTH = 232;

const NAV = [
  { href: "/", label: "Dashboard", icon: <DashboardIcon /> },
  { href: "/products", label: "Products", icon: <InventoryIcon /> },
  { href: "/decisions", label: "Decisions", icon: <AccountTreeIcon /> },
  { href: "/expressions", label: "Expressions", icon: <FunctionsIcon /> },
  { href: "/matrices", label: "Matrices", icon: <GridOnIcon /> },
  { href: "/rate-sheets", label: "Rate sheets", icon: <TableChartIcon /> },
  { href: "/publications", label: "Publications", icon: <HistoryIcon /> },
  { href: "/sandbox", label: "Sandbox", icon: <ScienceIcon /> },
  { href: "/grants", label: "Grants", icon: <AdminPanelSettingsIcon />, adminOnly: true },
  { href: "/audit", label: "Audit", icon: <FactCheckIcon />, adminOnly: true },
];

export default function AppShell({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();
  const [publishOpen, setPublishOpen] = useState(false);
  const { me, canPublish, isAdmin } = useMe();

  return (
    <Box sx={{ display: "flex", minHeight: "100vh" }}>
      <AppBar
        position="fixed"
        elevation={0}
        sx={{
          zIndex: (t) => t.zIndex.drawer + 1,
          bgcolor: "background.paper",
          borderBottom: 1,
          borderColor: "divider",
        }}
      >
        <Toolbar variant="dense" sx={{ gap: 2 }}>
          <Typography
            variant="h6"
            component={Link}
            href="/"
            sx={{ color: "primary.main", textDecoration: "none", fontWeight: 700 }}
          >
            MPEx
          </Typography>
          <Typography variant="body2" color="text.secondary" sx={{ flexGrow: 1 }}>
            Mortgage product eligibility &amp; pricing
          </Typography>
          {canPublish && (
            <Button
              variant="contained"
              size="small"
              startIcon={<PublishIcon />}
              onClick={() => setPublishOpen(true)}
            >
              Publish
            </Button>
          )}
          {me && (
            <Chip
              size="small"
              variant="outlined"
              label={`${me.email ?? me.sub} · ${me.roles.join(", ") || "no roles"}`}
            />
          )}
          <Tooltip title="Sign out">
            <IconButton size="small" component="a" href="/auth/logout">
              <LogoutIcon fontSize="small" />
            </IconButton>
          </Tooltip>
        </Toolbar>
      </AppBar>

      <Drawer
        variant="permanent"
        sx={{
          width: DRAWER_WIDTH,
          flexShrink: 0,
          "& .MuiDrawer-paper": {
            width: DRAWER_WIDTH,
            boxSizing: "border-box",
            bgcolor: "background.paper",
            borderColor: "divider",
          },
        }}
      >
        <Toolbar variant="dense" />
        <Divider />
        <List dense sx={{ px: 1 }}>
          {NAV.filter((item) => !("adminOnly" in item && item.adminOnly) || isAdmin).map((item) => {
            const selected =
              item.href === "/"
                ? pathname === "/"
                : pathname.startsWith(item.href);
            return (
              <ListItemButton
                key={item.href}
                component={Link}
                href={item.href}
                selected={selected}
                sx={{
                  borderRadius: 1,
                  mb: 0.5,
                  "&.Mui-selected": { bgcolor: "action.selected" },
                }}
              >
                <ListItemIcon sx={{ minWidth: 36, color: "inherit" }}>
                  {item.icon}
                </ListItemIcon>
                <ListItemText primary={item.label} />
              </ListItemButton>
            );
          })}
        </List>
      </Drawer>

      <Box component="main" sx={{ flexGrow: 1, p: 3, minWidth: 0 }}>
        <Toolbar variant="dense" />
        {children}
      </Box>

      <PublishDialog open={publishOpen} onClose={() => setPublishOpen(false)} />
    </Box>
  );
}
