// Client API minimaliste — toutes les routes sont servies via le proxy Vite
// `/api` -> http://localhost:3000 (voir vite.config.ts).
//
// Filtres optionnels (backend à implémenter) :
//   GET /api/bilan?level=...&scenario=REEL&period=2024
//   GET /api/compte-resultat?level=...&scenario=REEL&period=2024
//   GET /api/entries?level=...&scenario=REEL&period=2024&limit=...&offset=...
// `scenario` filtre sur e.scenario. `period` filtre sur entry_period
// (exercice clôturé — le seed pose entry_period = period = '2024').
// Si omis, pas de filtre.

import type {
  BilanRow,
  Entry,
  HealthStatus,
  LevelCount,
  MasterTable,
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

async function errorFromResponse(res: Response, label: string): Promise<Error> {
  let detail = '';
  try {
    const body = await res.json();
    if (body && typeof body === 'object' && 'error' in body) {
      detail = String((body as { error: unknown }).error);
    }
  } catch {
    // corps non JSON : on ignore
  }
  return new Error(detail || `${label} -> HTTP ${res.status}`);
}

async function postJsonRaw<T>(path: string, body: unknown): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw await errorFromResponse(res, `POST ${path}`);
  return (await res.json()) as T;
}

async function putJson<T>(path: string, body: unknown): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw await errorFromResponse(res, `PUT ${path}`);
  return (await res.json()) as T;
}

async function deleteJson<T>(path: string): Promise<T> {
  const res = await fetch(`${BASE}${path}`, { method: 'DELETE' });
  if (!res.ok) throw await errorFromResponse(res, `DELETE ${path}`);
  return (await res.json()) as T;
}

async function postForm<T>(path: string, form: FormData): Promise<T> {
  const res = await fetch(`${BASE}${path}`, { method: 'POST', body: form });
  if (!res.ok) throw await errorFromResponse(res, `POST ${path}`);
  return (await res.json()) as T;
}

export const api = {
  health: (signal?: AbortSignal) => getJson<HealthStatus>('/health', signal),
  levels: () => getJson<LevelCount[]>('/levels'),
  bilan: (level?: string, filters: { scenario?: string; period?: string } = {}) => {
    const search = new URLSearchParams();
    if (level) search.set('level', level);
    if (filters.scenario) search.set('scenario', filters.scenario);
    if (filters.period) search.set('period', filters.period);
    const qs = search.toString();
    return getJson<BilanRow[]>(`/bilan${qs ? `?${qs}` : ''}`);
  },
  compteResultat: (
    level?: string,
    filters: { scenario?: string; period?: string } = {},
  ) => {
    const search = new URLSearchParams();
    if (level) search.set('level', level);
    if (filters.scenario) search.set('scenario', filters.scenario);
    if (filters.period) search.set('period', filters.period);
    const qs = search.toString();
    return getJson<BilanRow[]>(`/compte-resultat${qs ? `?${qs}` : ''}`);
  },
  entries: (
    params: {
      level?: string;
      limit?: number;
      offset?: number;
      scenario?: string;
      period?: string;
    } = {},
  ) => {
    const search = new URLSearchParams();
    if (params.level) search.set('level', params.level);
    if (params.limit !== undefined) search.set('limit', String(params.limit));
    if (params.offset !== undefined) search.set('offset', String(params.offset));
    if (params.scenario) search.set('scenario', params.scenario);
    if (params.period) search.set('period', params.period);
    const qs = search.toString();
    return getJson<Entry[]>(`/entries${qs ? `?${qs}` : ''}`);
  },
  run: () => postJson<PipelineCounts>('/run'),
  reset: () => postJson<{ status: string; entries: number }>('/reset'),
  masterData: {
    list: (table: MasterTable) => getJson<unknown[]>(`/md/${table}`),
    create: (table: MasterTable, row: Record<string, unknown>) =>
      postJsonRaw<unknown>(`/md/${table}`, row),
    update: (table: MasterTable, row: Record<string, unknown>) =>
      putJson<unknown>(`/md/${table}`, row),
    remove: (table: MasterTable, pk: Record<string, string>) => {
      const qs = new URLSearchParams(pk).toString();
      return deleteJson<{ deleted: number }>(`/md/${table}?${qs}`);
    },
  },
  importEntries: (file: File) => {
    const form = new FormData();
    form.append('file', file);
    return postForm<{ imported: number }>('/import/entries', form);
  },
  importRates: (file: File) => {
    const form = new FormData();
    form.append('file', file);
    return postForm<{ imported: number }>('/import/rates', form);
  },
  importPerimeter: (file: File) => {
    const form = new FormData();
    form.append('file', file);
    return postForm<{ imported: number }>('/import/perimeter', form);
  },
};
