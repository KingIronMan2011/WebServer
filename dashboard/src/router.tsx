import { createBrowserRouter, Navigate } from "react-router";
import { AppShell } from "@/components/app-shell";
import { LoginPage } from "@/pages/login";
import { OverviewPage } from "@/pages/overview";
import { SitesPage } from "@/pages/sites";
import { OperationsPage } from "@/pages/operations";
import { AuditPage } from "@/pages/audit";
import { UsersPage } from "@/pages/users";

export const router = createBrowserRouter([
  { path: "/login", element: <LoginPage /> },
  {
    path: "/",
    element: <AppShell />,
    children: [
      { index: true, element: <OverviewPage /> },
      { path: "/sites", element: <SitesPage /> },
      { path: "/operations", element: <OperationsPage /> },
      { path: "/audit", element: <AuditPage /> },
      { path: "/users", element: <UsersPage /> },
    ],
  },
  { path: "*", element: <Navigate to="/" replace /> },
]);
