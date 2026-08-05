import { Button as BaseButton } from "@base-ui/react/button";
import type { ComponentProps } from "react";
import { cn } from "@/lib/utils";

export function Button({
  className,
  ...props
}: ComponentProps<typeof BaseButton>) {
  return (
    <BaseButton
      className={cn(
        "inline-flex h-10 items-center justify-center rounded-md bg-primary px-4 text-sm font-semibold text-slate-950 transition hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-50",
        className,
      )}
      {...props}
    />
  );
}
