import { useState } from "react";
import { useNavigate } from "react-router";
import { useForm } from "react-hook-form";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { KeyRound, ServerCog } from "lucide-react";
import { api } from "@/lib/api";
import { Button } from "@/components/ui/button";

const schema = z.object({
  username: z.string().min(1),
  password: z.string().min(1),
  setupCode: z.string().optional(),
});
type Form = z.infer<typeof schema>;

export function LoginPage() {
  const navigate = useNavigate();
  const [error, setError] = useState<string>();
  const form = useForm<Form>({ resolver: zodResolver(schema) });
  const submit = form.handleSubmit(
    async ({ username, password, setupCode }) => {
      try {
        await api.login(username, password, setupCode);
        navigate("/");
      } catch (reason) {
        setError(
          reason instanceof Error ? reason.message : "Anmeldung fehlgeschlagen",
        );
      }
    },
  );
  return (
    <main className="grid min-h-screen place-items-center p-5">
      <form
        onSubmit={submit}
        className="w-full max-w-sm rounded-xl border border-border bg-surface p-6 shadow-2xl"
      >
        <div className="mb-6 flex items-center gap-3">
          <div className="rounded-lg bg-primary p-2 text-slate-950">
            <ServerCog />
          </div>
          <div>
            <h1 className="font-semibold">Webserver Administration</h1>
            <p className="text-sm text-muted">Sicher anmelden</p>
          </div>
        </div>
        {(
          [
            ["username", "Benutzername", "text"],
            ["password", "Passwort", "password"],
            ["setupCode", "Setup-Code (nur Erstlogin)", "text"],
          ] as const
        ).map(([name, label, type]) => (
          <label key={name} className="mb-4 block text-sm">
            <span className="mb-1 block text-muted">{label}</span>
            <input
              type={type}
              autoComplete={
                name === "password" ? "current-password" : "username"
              }
              className="w-full rounded-md border border-border bg-black/20 px-3 py-2 outline-none focus:border-primary"
              {...form.register(name)}
            />
          </label>
        ))}
        {error && (
          <p className="mb-3 rounded-md bg-red-500/10 p-3 text-sm text-red-300">
            {error}
          </p>
        )}
        <Button className="w-full">
          <KeyRound size={16} /> Anmelden
        </Button>
      </form>
    </main>
  );
}
