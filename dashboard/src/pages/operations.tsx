import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { Page, Panel } from "@/components/page";

function Metric({ name, value }: { name: string; value: string | undefined }) {
  return (
    <div className="rounded-md bg-black/20 p-3">
      <p className="text-xs text-muted">{name}</p>
      <p className="mt-1 text-xl font-semibold">{value ?? "0"}</p>
    </div>
  );
}

export function OperationsPage() {
  const upstreams = useQuery({
    queryKey: ["upstreams"],
    queryFn: api.upstreams,
    refetchInterval: 10_000,
  });
  const certificates = useQuery({
    queryKey: ["certificates"],
    queryFn: api.certificates,
  });
  const metrics = useQuery({
    queryKey: ["metrics"],
    queryFn: api.metrics,
    refetchInterval: 10_000,
  });
  const observability = useQuery({
    queryKey: ["observability"],
    queryFn: api.observability,
  });
  const parsed = Object.fromEntries(
    (
      metrics.data?.prometheus.matchAll(
        /^(webserver_[^{\s]+)(?:\{[^}]+\})?\s+(\d+)/gm,
      ) ?? []
    ).map((match) => [match[1], match[2]]),
  );
  return (
    <Page
      title="Betrieb"
      description="Health Checks, Verbindungen, Zertifikate, Prometheus und Tracing."
    >
      <div className="grid gap-4 xl:grid-cols-2">
        <Panel>
          <h2 className="font-medium">Upstream-Health</h2>
          <div className="mt-4 space-y-3">
            {upstreams.data?.map((item) => (
              <div
                key={item.url}
                className="rounded-md bg-black/20 p-3 text-sm"
              >
                <div className="flex justify-between gap-4">
                  <p className="truncate">{item.url}</p>
                  <span
                    className={
                      item.circuit_open ? "text-red-300" : "text-emerald-300"
                    }
                  >
                    {item.circuit_open ? "Circuit offen" : "Gesund"}
                  </span>
                </div>
                <p className="mt-1 text-muted">
                  {item.active_connections} aktiv · {item.consecutive_failures}{" "}
                  Fehler
                </p>
              </div>
            )) ?? <p className="text-sm text-muted">Lade Status …</p>}
          </div>
        </Panel>
        <Panel>
          <h2 className="font-medium">Zertifikate</h2>
          <div className="mt-4 space-y-3">
            {certificates.data?.map((item) => (
              <div
                key={item.hosts.join(",")}
                className="rounded-md bg-black/20 p-3 text-sm"
              >
                <p>{item.hosts.join(", ")}</p>
                <p className="mt-1 text-muted">
                  {item.source.toUpperCase()} ·{" "}
                  {item.expires_at ?? "Ablaufdatum nicht verfügbar"}
                </p>
              </div>
            )) ?? <p className="text-sm text-muted">Lade Zertifikate …</p>}
          </div>
        </Panel>
        <Panel>
          <h2 className="font-medium">Prometheus-Metriken</h2>
          <div className="mt-4 grid grid-cols-2 gap-3">
            <Metric
              name="Requests gesamt"
              value={parsed.webserver_requests_total}
            />
            <Metric
              name="2xx-Antworten"
              value={parsed.webserver_responses_total}
            />
          </div>
          <p className="mt-3 text-xs text-muted">
            {observability.data?.prometheus.enabled
              ? `Exponiert unter ${observability.data.prometheus.path}`
              : "Kein öffentlicher Metrics-Pfad konfiguriert."}
          </p>
        </Panel>
        <Panel>
          <h2 className="font-medium">OpenTelemetry Tracing</h2>
          <p
            className={
              observability.data?.tracing.enabled
                ? "mt-4 text-emerald-300"
                : "mt-4 text-amber-300"
            }
          >
            {observability.data?.tracing.enabled
              ? "OTLP-Export aktiv"
              : "Kein OTLP-Export konfiguriert"}
          </p>
          <p className="mt-2 break-all text-sm text-muted">
            {observability.data?.tracing.endpoint ??
              "Setze OTEL_EXPORTER_OTLP_TRACES_ENDPOINT oder OTEL_EXPORTER_OTLP_ENDPOINT, um Traces zu exportieren."}
          </p>
        </Panel>
      </div>
    </Page>
  );
}
