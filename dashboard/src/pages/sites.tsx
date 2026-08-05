import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronDown, Plus, Trash2 } from "lucide-react";
import { useState } from "react";
import { api, type Route } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Page, Panel } from "@/components/page";

function routeFromForm(form: HTMLFormElement): Route {
  const values = new FormData(form);
  const kind = String(values.get("kind")) as Route["kind"];
  const route: Route = { path_prefix: String(values.get("path_prefix")), kind };
  if (kind === "static") {
    route.root = String(values.get("root"));
    route.index_file = String(values.get("index_file") || "index.html");
  } else if (kind === "proxy") {
    route.upstream = String(values.get("upstream"));
  } else {
    route.location = String(values.get("location"));
    route.status = Number(values.get("status") || 308);
  }
  return route;
}

function RouteForm({
  onSubmit,
  submitLabel,
}: {
  onSubmit: (route: Route) => void;
  submitLabel: string;
}) {
  const [kind, setKind] = useState<Route["kind"]>("static");
  return (
    <form
      className="mt-4 grid gap-3 rounded-lg border border-border bg-black/15 p-4 md:grid-cols-2"
      onSubmit={(event) => {
        event.preventDefault();
        onSubmit(routeFromForm(event.currentTarget));
        event.currentTarget.reset();
      }}
    >
      <input
        name="path_prefix"
        required
        defaultValue="/"
        className="field"
        placeholder="Pfad, z. B. /api"
      />
      <select
        name="kind"
        className="field"
        value={kind}
        onChange={(event) => setKind(event.target.value as Route["kind"])}
      >
        <option value="static">Statische Dateien</option>
        <option value="proxy">Reverse Proxy</option>
        <option value="redirect">Weiterleitung</option>
      </select>
      {kind === "static" && (
        <>
          <input
            name="root"
            required
            defaultValue="./public"
            className="field"
            placeholder="Datei-Wurzel"
          />
          <input
            name="index_file"
            defaultValue="index.html"
            className="field"
            placeholder="Index-Datei"
          />
        </>
      )}
      {kind === "proxy" && (
        <input
          name="upstream"
          required
          className="field md:col-span-2"
          placeholder="http://127.0.0.1:3000"
        />
      )}
      {kind === "redirect" && (
        <>
          <input
            name="location"
            required
            className="field"
            placeholder="https://example.com/"
          />
          <select name="status" className="field">
            <option value="308">308 Permanent</option>
            <option value="302">302 Temporär</option>
          </select>
        </>
      )}
      <Button type="submit" className="md:col-span-2">
        <Plus size={16} /> {submitLabel}
      </Button>
    </form>
  );
}

function SiteRoutes({ host }: { host: string }) {
  const client = useQueryClient();
  const routes = useQuery({
    queryKey: ["routes", host],
    queryFn: () => api.routes(host),
  });
  const create = useMutation({
    mutationFn: (route: Route) => api.createRoute(host, route),
    onSuccess: () => client.invalidateQueries({ queryKey: ["routes", host] }),
  });
  const remove = useMutation({
    mutationFn: (path: string) => api.deleteRoute(host, path),
    onSuccess: () => {
      client.invalidateQueries({ queryKey: ["routes", host] });
      client.invalidateQueries({ queryKey: ["sites"] });
    },
  });
  return (
    <div className="border-t border-border px-4 py-4">
      <div className="space-y-2">
        {routes.data?.map((route) => (
          <div
            key={route.path_prefix}
            className="flex items-center justify-between rounded-md bg-black/20 px-3 py-2 text-sm"
          >
            <span>
              <b>{route.path_prefix}</b> · {route.kind}
            </span>
            <Button
              aria-label={`Route ${route.path_prefix} löschen`}
              className="h-8 bg-red-400/15 px-2 text-red-200 hover:bg-red-400/25"
              onClick={() => {
                if (confirm(`Route ${route.path_prefix} wirklich löschen?`))
                  remove.mutate(route.path_prefix);
              }}
            >
              <Trash2 size={15} />
            </Button>
          </div>
        ))}
      </div>
      <RouteForm
        submitLabel="Route hinzufügen"
        onSubmit={(route) => create.mutate(route)}
      />
      {(create.error || remove.error) && (
        <p className="mt-3 text-sm text-red-300">
          Änderung konnte nicht gespeichert werden.
        </p>
      )}
    </div>
  );
}

export function SitesPage() {
  const client = useQueryClient();
  const sites = useQuery({ queryKey: ["sites"], queryFn: api.sites });
  const [createOpen, setCreateOpen] = useState(false);
  const [host, setHost] = useState("");
  const [expanded, setExpanded] = useState<string | null>(null);
  const create = useMutation({
    mutationFn: ({ host, route }: { host: string; route: Route }) =>
      api.createSite(host, route),
    onSuccess: () => {
      client.invalidateQueries({ queryKey: ["sites"] });
      setCreateOpen(false);
    },
  });
  const remove = useMutation({
    mutationFn: api.deleteSite,
    onSuccess: () => client.invalidateQueries({ queryKey: ["sites"] }),
  });
  return (
    <Page
      title="Sites & Routen"
      description="Virtuelle Hosts, statische Inhalte, Proxies und Weiterleitungen verwalten."
    >
      <div className="mb-4 flex justify-end">
        <Button onClick={() => setCreateOpen((value) => !value)}>
          <Plus size={16} /> Site anlegen
        </Button>
      </div>
      {createOpen && (
        <Panel>
          <input
            value={host}
            onChange={(event) => setHost(event.target.value)}
            required
            className="field"
            placeholder="admin.example.com"
            aria-label="Hostname der neuen Site"
          />
          <RouteForm
            submitLabel="Site mit Start-Route erstellen"
            onSubmit={(route) => {
              if (host) create.mutate({ host, route });
            }}
          />
          {!host && (
            <p className="mt-3 text-xs text-muted">
              Gib zuerst den Hostnamen ein.
            </p>
          )}
          {create.error && (
            <p className="mt-3 text-sm text-red-300">
              Site konnte nicht erstellt werden. Prüfe Host und Route.
            </p>
          )}
        </Panel>
      )}
      <Panel>
        {sites.data?.length ? (
          <div className="divide-y divide-border">
            {sites.data.map((site) => (
              <div key={site.host}>
                <div className="flex items-center justify-between gap-3 py-4">
                  <button
                    className="flex min-w-0 items-center gap-2 text-left"
                    onClick={() =>
                      setExpanded(expanded === site.host ? null : site.host)
                    }
                  >
                    <ChevronDown
                      size={16}
                      className={
                        expanded === site.host
                          ? "rotate-180 transition"
                          : "transition"
                      }
                    />
                    <span>
                      <p className="font-medium">{site.host}</p>
                      <p className="text-sm text-muted">{site.routes} Routen</p>
                    </span>
                  </button>
                  <Button
                    aria-label={`${site.host} löschen`}
                    className="h-9 bg-red-400/15 px-3 text-red-200 hover:bg-red-400/25"
                    onClick={() => {
                      if (confirm(`Site ${site.host} wirklich löschen?`))
                        remove.mutate(site.host);
                    }}
                  >
                    <Trash2 size={16} />
                  </Button>
                </div>
                {expanded === site.host && <SiteRoutes host={site.host} />}
              </div>
            ))}
          </div>
        ) : (
          <p className="text-sm text-muted">Keine Sites vorhanden.</p>
        )}
      </Panel>
    </Page>
  );
}
