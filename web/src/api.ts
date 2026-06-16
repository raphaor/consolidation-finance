// Client API minimaliste — toutes les routes sont servies via le proxy Vite
// `/api` -> http://localhost:3000 (voir vite.config.ts).

import type {
  BilanRow,
  Entry,
  HealthStatus,
  LevelCount,
  PipelineCounts,
} from './types';

const BASE = '/api';

async function getJson<T>(path: string, signal?: AbortSignal): Promise<T> {
  const res = await fetch(`${BASE}${path}`, { signal });
  if (!res.ok) {
    throw new Error(`GET ${path} -> HTTP ${res.status}`);
  }
  return (await res.json()) as T;
}

async function postJson<T>(path: string): Promise<T> {
  const res = await fetch(`${BASE}${path}`, { method: 'POST' });
  if (!res.ok) {
    throw new Error(`POST ${path} -> HTTP ${res.status}`);
  }
  return (await res.json()) as T;
}

export const api = {
  health: (signal?: AbortSignal) => getJson<HealthStatus>('/health', signal),
  levels: () => getJson<LevelCount[]>('/levels'),
  bilan: (level?: string) => {
    const qs = level ? `?level=${encodeURIComponent(level)}` : '';
    return getJson<BilanRow[]>(`/bilan${qs}`);
  },
  entries: (params: { level?: string; limit?: number; offset?: number } = {}) => {
    const search = new URLSearchParams();
    if (params.level) search.set('level', params.level);
    if (params.limit !== undefined) search.set('limit', String(params.limit));
    if (params.offset !== undefined) search.set('offset', String(params.offset));
    const qs = search.toString();
    return getJson<Entry[]>(`/entries${qs ? `?${qs}` : ''}`);
  },
  run: () => postJson<PipelineCounts>('/run'),
  reset: () => postJson<{ status: string; entries: number }>('/reset'),
};
