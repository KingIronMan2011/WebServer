import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { Page, Panel } from "@/components/page";

export function UsersPage() {
  const session = useQuery({ queryKey: ["session"], queryFn: api.me });
  const users = useQuery({
    queryKey: ["users"],
    queryFn: api.users,
    enabled: session.data?.role === "admin",
  });
  const client = useQueryClient();
  const update = useMutation({
    mutationFn: ({
      username,
      role,
    }: {
      username: string;
      role: "admin" | "operator" | "viewer";
    }) => api.updateRole(username, role),
    onSuccess: () => client.invalidateQueries({ queryKey: ["users"] }),
  });
  return (
    <Page
      title="Benutzer"
      description="Rollen und Zugriff auf die Verwaltungsoberfläche."
    >
      <Panel>
        {session.data?.role === "admin" ? (
          <div className="divide-y divide-border">
            {users.data?.map((user) => (
              <div
                key={user.username}
                className="flex items-center justify-between gap-3 py-3"
              >
                <span>{user.username}</span>
                <select
                  value={user.role}
                  onChange={(event) =>
                    update.mutate({
                      username: user.username,
                      role: event.target.value as
                        "admin" | "operator" | "viewer",
                    })
                  }
                  className="rounded-md border border-border bg-black/20 px-2 py-1 text-sm"
                >
                  <option value="admin">Admin</option>
                  <option value="operator">Operator</option>
                  <option value="viewer">Viewer</option>
                </select>
              </div>
            ))}
          </div>
        ) : (
          <p className="text-sm text-muted">
            Für diese Ansicht ist die Rolle Administrator erforderlich.
          </p>
        )}
      </Panel>
    </Page>
  );
}
