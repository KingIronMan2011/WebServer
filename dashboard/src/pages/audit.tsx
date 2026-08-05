import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { Page, Panel } from "@/components/page";

export function AuditPage() {
  const events = useQuery({ queryKey: ["audit"], queryFn: api.audit });
  return (
    <Page
      title="Audit-Log"
      description="Nachvollziehbare schreibende Verwaltungsaktionen."
    >
      <Panel>
        <div className="overflow-x-auto">
          <table className="w-full text-left text-sm">
            <thead className="text-muted">
              <tr>
                <th className="pb-3">Zeit</th>
                <th>Aktion</th>
                <th>Ziel</th>
                <th>Quelle</th>
              </tr>
            </thead>
            <tbody>
              {events.data?.map((event) => (
                <tr
                  key={`${event.created_at}-${event.action}`}
                  className="border-t border-border"
                >
                  <td className="py-3">
                    {new Date(event.created_at * 1000).toLocaleString()}
                  </td>
                  <td>{event.action}</td>
                  <td>{event.target}</td>
                  <td>{event.source_ip}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </Panel>
    </Page>
  );
}
