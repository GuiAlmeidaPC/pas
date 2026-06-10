export interface Tab {
  id: string;
  path: string | null;
  title: string;
  content: string;
  saved_content: string; // baseline for dirty detection
}

export function makeTab(opts: {
  id?: string;
  path?: string | null;
  title?: string;
  content: string;
}): Tab {
  const id =
    opts.id ?? (typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID()
      : `tab-${Date.now()}-${Math.random()}`);
  return {
    id,
    path: opts.path ?? null,
    title: opts.title ?? "untitled.pas",
    content: opts.content,
    saved_content: opts.content,
  };
}

export function basename(p: string): string {
  const parts = p.split(/[\\/]/);
  return parts[parts.length - 1] || p;
}
