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
  DataHealthReport,
  DimensionInfo,
  HealthStatus,
  LevelCount,
  MasterTable,
  PipelineCounts,
  ReferenceInfo,
  ReportFilters,
  RuleDetail,
  RuleSummary,
  RulesetDetail,
  RulesetSummary,
  ScenarioSummary,
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
    throw new Error(`GET ${BASE}${path} -> HTTP ${res.status} ${res.statusText}`.trim());
  }
  return (await res.json()) as T;
}

async function postJson<T>(path: string): Promise<T> {
  const res = await fetch(`${BASE}${path}`, { method: 'POST' });
  if (!res.ok) {
    throw new Error(`POST ${BASE}${path} -> HTTP ${res.status} ${res.statusText}`.trim());
  }
  return (await res.json()) as T;
}

async function errorFromResponse(res: Response, label: string): Promise<Error> {
  let detail = '';
  // Tente d'abord un corps JSON structuré `{ "error": "..." }` (réponses métier
  // 4xx produites par l'application).
  try {
    const body = await res.json();
    if (body && typeof body === 'object' && 'error' in body) {
      detail = String((body as { error: unknown }).error);
    }
  } catch {
    // Corps non-JSON : tente le texte brut (Axum répond souvent en HTML ou texte
    // brut pour les erreurs de routing comme 404/405 — utile pour le diagnostic).
    try {
      const text = await res.text();
      if (text) detail = text.slice(0, 300);
    } catch {
      // vraiment vide : on garde detail=''
    }
  }
  const suffix = res.statusText ? ` ${res.statusText}` : '';
  return new Error(detail || `${label} -> HTTP ${res.status}${suffix}`.trim());
}

async function postJsonRaw<T>(path: string, body: unknown): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw await errorFromResponse(res, `POST ${BASE}${path}`);
  return (await res.json()) as T;
}

async function putJson<T>(path: string, body: unknown): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw await errorFromResponse(res, `PUT ${BASE}${path}`);
  return (await res.json()) as T;
}

async function deleteJson<T>(path: string): Promise<T> {
  const res = await fetch(`${BASE}${path}`, { method: 'DELETE' });
  if (!res.ok) throw await errorFromResponse(res, `DELETE ${BASE}${path}`);
  return (await res.json()) as T;
}

async function postForm<T>(path: string, form: FormData): Promise<T> {
  const res = await fetch(`${BASE}${path}`, { method: 'POST', body: form });
  if (!res.ok) throw await errorFromResponse(res, `POST ${BASE}${path}`);
  return (await res.json()) as T;
}

export const api = {
  health: (signal?: AbortSignal) => getJson<HealthStatus>('/health', signal),
  levels: () => getJson<LevelCount[]>('/levels'),
  bilan: (level: string, filters?: ReportFilters) =>
    getJson<BilanRow[]>(`/bilan${buildQueryString({ level, ...filters })}`),
  compteResultat: (level: string, filters?: ReportFilters) =>
    getJson<BilanRow[]>(`/compte-resultat${buildQueryString({ level, ...filters })}`),
  // Colonnes dynamiques (dimensions built-in + custom + level + amount) →
  // chaque ligne est un objet générique ; la vue Écritures construit ses
  // colonnes depuis /api/meta/dimensions.
  entries: (params: { level: string; limit?: number; offset?: number } & ReportFilters) =>
    getJson<Record<string, unknown>[]>(`/entries${buildQueryString(params)}`),
  run: (scenario?: string) =>
    scenario && scenario.trim() !== ''
      ? postJsonRaw<PipelineCounts>('/run', { scenario })
      : postJson<PipelineCounts>('/run'),
  reset: () => postJson<{ status: string; entries: number }>('/reset'),
  // Sauvegarde / restauration : paquet JSON complet de l'état (référentiels +
  // écritures + règles + dimensions custom). `importAll` remplace tout.
  backup: {
    exportAll: () => getJson<Record<string, unknown>>('/export'),
    importAll: (bundle: unknown) =>
      postJsonRaw<{ status: string; imported: Record<string, number> }>(
        '/import/all',
        bundle,
      ),
  },
  scenarios: {
    list: () => getJson<ScenarioSummary[]>('/scenarios'),
  },
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
  rules: {
    list: () => getJson<RuleSummary[]>('/rules'),
    get: (code: string) => getJson<RuleDetail>(`/rules/${code}`),
    create: (body: { code: string; libelle: string; definition: object }) =>
      postJsonRaw<RuleDetail>('/rules', body),
    // Le serveur attend `code` dans le corps (RuleBody.code, validé == URL).
    update: (code: string, body: { libelle?: string; definition?: object }) =>
      putJson<RuleDetail>(`/rules/${code}`, { code, ...body }),
    remove: (code: string) => deleteJson<{ deleted: number }>(`/rules/${code}`),
  },
  rulesets: {
    list: () => getJson<RulesetSummary[]>('/rulesets'),
    get: (code: string) => getJson<RulesetDetail>(`/rulesets/${code}`),
    create: (
      body: {
        code: string;
        libelle: string;
        items: { ordre: number; rule_code: string }[];
      },
    ) => postJsonRaw<RulesetDetail>('/rulesets', body),
    update: (
      code: string,
      body: {
        libelle?: string;
        items?: { ordre: number; rule_code: string }[];
      },
    ) => putJson<RulesetDetail>(`/rulesets/${code}`, { code, ...body }),
    remove: (code: string) =>
      deleteJson<{ deleted: number }>(`/rulesets/${code}`),
  },
  // Graphe des références (source de vérité serveur), pour les dropdowns
  // contextuels — remplace les miroirs codés en dur côté front.
  references: () => getJson<ReferenceInfo[]>('/meta/references'),
  // Rapport « santé des données » : orphelins sur tout le graphe de références.
  dataHealth: () => getJson<DataHealthReport>('/meta/health'),
  dimensions: {
    list: () => getJson<DimensionInfo[]>('/meta/dimensions'),
    create: (body: { name: string; label: string }) =>
      postJsonRaw<DimensionInfo>('/meta/dimensions', body),
    remove: (name: string) =>
      deleteJson<{ deleted: number }>(`/meta/dimensions/${name}`),
  },
};
