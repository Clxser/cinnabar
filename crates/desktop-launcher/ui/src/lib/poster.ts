// Generated poster art for servers that ship no artwork of their own.

export function hashString(text: string): number {
  let h = 2166136261;
  for (let i = 0; i < text.length; i++) h = Math.imul(h ^ text.charCodeAt(i), 16777619);
  return h >>> 0;
}

/** Hues chosen to sit well against the dusk palette; a server's name picks one. */
const HUES = [8, 24, 158, 188, 212, 236, 268, 322];

export interface Poster {
  hue: number;
  initial: string;
}

export function posterFor(name: string): Poster {
  const h = hashString(name);
  const initial = name.trim().charAt(0).toUpperCase() || '?';
  return { hue: HUES[h % HUES.length]!, initial };
}
