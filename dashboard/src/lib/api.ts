export type Role = "admin" | "operator" | "viewer";
export type Site = { host: string; routes: number };
export type Upstream = {
  url: string;
  active_connections: number;
  consecutive_failures: number;
  circuit_open: boolean;
};
export type Certificate = {
  hosts: string[];
  source: "local" | "acme";
  expires_at: string | null;
};
export type AuditEvent = {
  created_at: number;
  user_id: string | null;
  source_ip: string;
  action: string;
  target: string;
  success: boolean;
};
export type User = { username: string; role: Role };
export type Route = {
  path_prefix: string;
  kind: "static" | "proxy" | "redirect";
  root?: string;
  index_file?: string;
  upstream?: string;
  upstreams?: Array<string | { url: string; weight?: number }>;
  location?: string;
  status?: number;
};
export type Observability = {
  tracing: { enabled: boolean; exporter: string; endpoint: string | null };
  prometheus: { enabled: boolean; path: string | null };
};

export class ApiError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    credentials: "include",
    headers: { "content-type": "application/json", ...init?.headers },
    ...init,
  });
  if (!response.ok) {
    const body = (await response
      .json()
      .catch(() => ({ message: response.statusText }))) as { message?: string };
    throw new ApiError(
      response.status,
      body.message ?? "Request fehlgeschlagen",
    );
  }
  if (response.status === 204) return undefined as T;
  return response.json() as Promise<T>;
}

export const api = {
  me: () => request<{ role: Role }>("/api/v1/auth/me"),
  login: (username: string, password: string, setupCode?: string) =>
    request<{ password_change_required: boolean; role: Role }>(
      "/api/v1/auth/login",
      {
        method: "POST",
        body: JSON.stringify({ username, password, setup_code: setupCode }),
      },
    ),
  logout: () => request<void>("/api/v1/auth/logout", { method: "POST" }),
  sites: () => request<Site[]>("/api/v1/sites"),
  createSite: (host: string, route: Route) =>
    request<void>("/api/v1/sites", {
      method: "POST",
      body: JSON.stringify({ host, route }),
    }),
  deleteSite: (host: string) =>
    request<void>(`/api/v1/sites/${encodeURIComponent(host)}`, {
      method: "DELETE",
    }),
  routes: (host: string) =>
    request<Route[]>(`/api/v1/sites/${encodeURIComponent(host)}/routes`),
  createRoute: (host: string, route: Route) =>
    request<void>(`/api/v1/sites/${encodeURIComponent(host)}/routes`, {
      method: "POST",
      body: JSON.stringify(route),
    }),
  deleteRoute: (host: string, pathPrefix: string) =>
    request<void>(
      `/api/v1/sites/${encodeURIComponent(host)}/routes?path_prefix=${encodeURIComponent(pathPrefix)}`,
      { method: "DELETE" },
    ),
  upstreams: () => request<Upstream[]>("/api/v1/upstreams"),
  certificates: () => request<Certificate[]>("/api/v1/certificates"),
  audit: () => request<AuditEvent[]>("/api/v1/logs"),
  metrics: () => request<{ prometheus: string }>("/api/v1/metrics"),
  observability: () => request<Observability>("/api/v1/observability"),
  users: () => request<User[]>("/api/v1/users"),
  updateRole: (username: string, role: Role) =>
    request<void>(`/api/v1/users/${encodeURIComponent(username)}/role`, {
      method: "POST",
      body: JSON.stringify({ role }),
    }),
};
