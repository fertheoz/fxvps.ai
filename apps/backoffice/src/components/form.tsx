"use client";
import * as React from "react";
import type { z } from "zod";
import { FieldError, Input, Label, Select } from "@/components/ui/primitives";

/** Small helper around a draft object validated by a zod schema. */
export function useZodForm<T extends object>(schema: z.ZodType<T>, initial: T) {
  const [draft, setDraft] = React.useState<T>(initial);
  const [errors, setErrors] = React.useState<Record<string, string>>({});
  const set = <K extends keyof T>(k: K, v: T[K]) => setDraft((d) => ({ ...d, [k]: v }));
  const validate = (): T | null => {
    const r = schema.safeParse(draft);
    if (r.success) {
      setErrors({});
      return r.data;
    }
    setErrors(Object.fromEntries(r.error.issues.map((i) => [String(i.path[0] ?? "_"), i.message])));
    return null;
  };
  return { draft, set, errors, validate };
}

export function NumField({ label, value, onChange, error, step, disabled }: { label: string; value: number; onChange: (n: number) => void; error?: string; step?: number; disabled?: boolean }) {
  return (
    <Label>
      {label}
      <Input type="number" step={step ?? "any"} value={Number.isFinite(value) ? value : ""} disabled={disabled} onChange={(e) => onChange(e.target.value === "" ? NaN : Number(e.target.value))} />
      <FieldError msg={error} />
    </Label>
  );
}

export function TextField({ label, value, onChange, error, disabled }: { label: string; value: string; onChange: (s: string) => void; error?: string; disabled?: boolean }) {
  return (
    <Label>
      {label}
      <Input value={value} disabled={disabled} onChange={(e) => onChange(e.target.value)} />
      <FieldError msg={error} />
    </Label>
  );
}

export function SelectField<T extends string>({ label, value, options, onChange, disabled }: { label: string; value: T; options: readonly T[]; onChange: (v: T) => void; disabled?: boolean }) {
  return (
    <Label>
      {label}
      <Select value={value} disabled={disabled} onChange={(e) => onChange(e.target.value as T)}>
        {options.map((o) => <option key={o} value={o}>{o}</option>)}
      </Select>
    </Label>
  );
}
