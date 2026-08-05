import {
  Activity,
  FileCog,
  LayoutDashboard,
  LogOut,
  ScrollText,
  ServerCog,
  Users,
} from "lucide-react";
import { NavLink, Outlet, useNavigate } from "react-router";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

const navigation = [
  ["/", "Übersicht", LayoutDashboard],
  ["/sites", "Sites & Routen", FileCog],
  ["/operations", "Betrieb", Activity],
  ["/audit", "Audit-Log", ScrollText],
  ["/users", "Benutzer", Users],
] as const;

export function AppShell() {
  const navigate = useNavigate();
  const session = useQuery({
    queryKey: ["session"],
    queryFn: api.me,
    retry: false,
  });
  if (session.error) {
    navigate("/login", { replace: true });
    return null;
  }
  return (
    <div className="min-h-screen lg:grid lg:grid-cols-[17rem_1fr]">
      <aside className="border-b border-border bg-surface p-5 lg:border-r lg:border-b-0">
        <div className="mb-8 flex items-center gap-3">
          <div className="rounded-lg bg-primary p-2 text-slate-950">
            <ServerCog size={22} />
          </div>
          <div>
            <p className="font-semibold">Webserver</p>
            <p className="text-xs text-muted">Administration</p>
          </div>
        </div>
        <nav className="flex gap-2 overflow-x-auto lg:flex-col">
          {navigation.map(([to, label, Icon]) => (
            <NavLink
              key={to}
              to={to}
              className={({ isActive }) =>
                cn(
                  "flex items-center gap-3 rounded-md px-3 py-2 text-sm text-muted hover:bg-white/5 hover:text-white",
                  isActive && "bg-white/10 text-white",
                )
              }
            >
              <Icon size={17} />
              {label}
            </NavLink>
          ))}
        </nav>
        <div className="mt-8 border-t border-border pt-4">
          <p className="mb-3 text-xs text-muted">
            Rolle: {session.data?.role ?? "…"}
          </p>
          <Button
            className="w-full bg-white/10 text-white hover:bg-white/15"
            onClick={async () => {
              await api.logout();
              navigate("/login");
            }}
          >
            <LogOut size={16} /> Abmelden
          </Button>
        </div>
      </aside>
      <main className="p-5 md:p-8">
        <Outlet />
      </main>
    </div>
  );
}
