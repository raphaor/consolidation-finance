// Client API minimaliste — toutes les routes sont servies via le proxy Vite
// `/api` -> http://localhost:3000 (voir vite.config.ts).
//
// Filtres optionnels (4, tous des strings, absents/vides = pas de filtre) :
//   GET /api/bilan?level=...&scenario=...&entity=...&entry_period=...&period=...
//   GET /api/compte-resultat?level=...&scenario=...&entity=...&entry_period=...&period=...
//   GET /api/entries?level=...&scenario=...&entity=...&entry_period=...&period=...&limit=...&offset=...
// - `scenario` -> scénario (REEL…)
// - `entity` -> entité juridique (M, A, B…)
// - `entry_period` -> exercice clôturé (2024)
// - `period` -> période impactée par l'écriture (peut différer de entry_period)

import type {
  BilanRow,
  Entry,
  HealthStatus,
  LevelCount,
  MasterTable,
  PipelineCounts,
  ReportFilters,
} from './types';

const BASE = '/api';

function buildQueryString(params: object): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined || value === '') continue;
    search.set(key, String(value));
  }
  const qs = search.toString();
  return qs ? `?${qs}` : '';
}

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
  bilan: (level: string, filters?: ReportFilters) =>
    getJson<BilanRow[]>(`/bilan${buildQueryString({ level, ...filters })}`),
  compteResultat: (level: string, filters?: ReportFilters) =>
    getJson<BilanRow[]>(`/compte-resultat${buildQueryString({ level, ...filters })}`),
  entries: (params: { level: string; limit?: number; offset?: number } & ReportFilters) =>
    getJson<Entry[]>(`/entries${buildQueryString(params)}`),
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
