import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { Page, Panel } from "@/components/page";

export function OverviewPage() {
  const sites = useQuery({ queryKey: ["sites"], queryFn: api.sites });
  const upstreams = useQuery({
    queryKey: ["upstreams"],
    queryFn: api.upstreams,
    refetchInterval: 10_000,
  });
  const certificates = useQuery({
    queryKey: ["certificates"],
    queryFn: api.certificates,
  });
  const stats = [
    ["Sites", sites.data?.length ?? "…"],
    ["Upstreams", upstreams.data?.length ?? "…"],
    ["Zertifikate", certificates.data?.length ?? "…"],
  ];
  return (
    <Page
      title="Übersicht"
      description="Status der Webserver-Verwaltung in Echtzeit."
    >
      <div className="grid gap-4 sm:grid-cols-3">
        {stats.map(([label, value]) => (
          <Panel key={label}>
            <p className="text-sm text-muted">{label}</p>
            <p className="mt-2 text-3xl font-semibold">{value}</p>
          </Panel>
        ))}
      </div>
      <Panel>
        <h2 className="font-medium">Upstream-Zustand</h2>
        <div className="mt-4 space-y-3">
          {upstreams.data?.map((upstream) => (
            <div
              key={upstream.url}
              className="flex items-center justify-between text-sm"
            >
              <span>{upstream.url}</span>
              <span
                className={
                  upstream.circuit_open ? "text-red-300" : "text-emerald-300"
                }
              >
                {upstream.circuit_open ? "Circuit offen" : "Verfügbar"}
              </span>
            </div>
          )) ?? <p className="text-sm text-muted">Lade Status …</p>}
        </div>
      </Panel>
    </Page>
  );
}
