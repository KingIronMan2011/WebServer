import type { ReactNode } from "react";

export function Page({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: ReactNode;
}) {
  return (
    <>
      <header className="mb-7">
        <h1 className="text-2xl font-semibold">{title}</h1>
        <p className="mt-1 text-sm text-muted">{description}</p>
      </header>
      {children}
    </>
  );
}

export function Panel({ children }: { children: ReactNode }) {
  return (
    <section className="rounded-xl border border-border bg-surface p-5">
      {children}
    </section>
  );
}
