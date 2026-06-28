// Client API minimaliste — toutes les routes sont servies via le proxy Vite
// `/api` -> http://localhost:3000 (voir vite.config.ts).
//
// Filtres optionnels (absents/vides = pas de filtre) :
//   GET /api/bilan?level=...&consolidation=...&entity=...&entry_period=...&period=...
//   GET /api/compte-resultat?level=...&consolidation=...&entity=...&entry_period=...&period=...
//   GET /api/entries?level=...&consolidation=...&phase=...&entity=...&entry_period=...&period=...&limit=...&offset=...
// - `consolidation` -> identifiant (number) de la consolidation (filtre des
//   niveaux fact : corporate/converted/consolidated)
// - `phase` -> phase (REEL…) pour le niveau raw (stg_entry) ; ignoré aux autres
// - `entity` -> entité juridique (M, A, B…)
// - `entry_period` -> exercice clôturé (2024)
// - `period` -> période impactée par l'écriture (peut différer de entry_period)

import type {
  BilanRow,
  Characteristic,
  Coefficient,
  CoefficientOperand,
  CoefficientPreview,
  Control,
  ControlDefinition,
  ControlOperand,
  ControlReport,
  ControlSet,
  ControlSetReport,
  CustomReference,
  DataHealthReport,
  Aggregate,
  Indicator,
  IndicatorOperand,
  IndicatorPreview,
  DimensionInfo,
  EntryInput,
  HealthStatus,
  LevelCount,
  MasterTable,
  NativeEnum,
  PipelineRunResult,
  ReferenceInfo,
  ReportFilters,
  RuleDetail,
  RuleSummary,
  RulesetDetail,
  RulesetSummary,
  ConsolidationSummary,
  TableSchema,
  TableSummary,
  ValueList,
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
  entries: (params: { level: string; limit?: number; offset?: number; phase?: string } & ReportFilters & { source?: string }) =>
    getJson<Record<string, unknown>[]>(`/entries${buildQueryString(params)}`),
  // Mutations unitaires / batch sur stg_entry (saisie manuelle). Toute ligne
  // créée via create() est marquée `source = MANUAL` côté back ; edit/delete
  // sont refusés sur les lignes importées par CSV (source ≠ MANUAL).
  entriesMutations: {
    create: (rows: EntryInput[]) =>
      postJsonRaw<{ inserted: number; ids: number[] }>('/entries', { rows }),
    update: (id: number, row: EntryInput) =>
      putJson<{ updated: number; id: number }>(`/entries/${id}`, row),
    remove: (id: number) =>
      deleteJson<{ deleted: number; id: number }>(`/entries/${id}`),
  },
  run: (consolidationId?: number) =>
    consolidationId !== undefined
      ? postJsonRaw<PipelineRunResult>('/run', { consolidation_id: consolidationId })
      : postJson<PipelineRunResult>('/run'),
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
  consolidations: {
    list: () => getJson<ConsolidationSummary[]>('/consolidations'),
  },
  masterData: {
    // Liste des tables navigables (natives + `car_<code>` + `lst_<code>`).
    listTables: () => getJson<TableSummary[]>('/md'),
    // Schéma complet d'une table : colonnes natives + dynamiques avec FK.
    schema: (table: MasterTable) => getJson<TableSchema>(`/md/${table}/schema`),
    list: (table: MasterTable) => getJson<unknown[]>(`/md/${table}`),
    create: (table: MasterTable, row: Record<string, unknown>) =>
      postJsonRaw<unknown>(`/md/${table}`, row),
    update: (table: MasterTable, row: Record<string, unknown>) =>
      putJson<unknown>(`/md/${table}`, row),
    remove: (table: MasterTable, pk: Record<string, string>) => {
      const qs = new URLSearchParams(pk).toString();
      return deleteJson<{ deleted: number }>(`/md/${table}?${qs}`);
    },
    // Renommage de code (chantier B1, étape 7) : possible uniquement si plus
    // aucune référence ne pointe vers le code (sinon le serveur refuse en
    // listant les blocages).
    rename: (table: MasterTable, oldCode: string, newCode: string) =>
      postJsonRaw<{ renamed: { old: string; new: string } }>(`/md/${table}/rename`, {
        old: oldCode,
        new: newCode,
      }),
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
  // Coefficients (moteur de formules — volet 1). Bibliothèque de formules
  // nommées (natives + utilisateur) consommées par le coefficient d'une règle.
  // `preview` valide + évalue une formule sans la sauvegarder (preview live).
  coefficients: {
    list: () => getJson<Coefficient[]>('/coefficients'),
    operands: () => getJson<CoefficientOperand[]>('/coefficients/operands'),
    preview: (body: { expression: string; samples?: Record<string, number> }) =>
      postJsonRaw<CoefficientPreview>('/coefficients/preview', body),
    create: (body: { code: string; libelle?: string; expression: string }) =>
      postJsonRaw<Coefficient>('/coefficients', body),
    update: (code: string, body: { libelle?: string; expression: string }) =>
      putJson<Coefficient>(`/coefficients/${code}`, { code, ...body }),
    remove: (code: string) => deleteJson<{ deleted: string }>(`/coefficients/${code}`),
  },
  // Indicateurs / KPI (moteur de formules — volet 2). Postes (agrégats nommés)
  // + indicateurs (formules combinant des postes), calculés au grain.
  aggregates: {
    list: () => getJson<Aggregate[]>('/aggregates'),
    create: (body: { code: string; libelle?: string; level: string; definition: object }) =>
      postJsonRaw<Aggregate>('/aggregates', body),
    update: (code: string, body: { libelle?: string; level: string; definition: object }) =>
      putJson<Aggregate>(`/aggregates/${code}`, { code, ...body }),
    remove: (code: string) => deleteJson<{ deleted: string }>(`/aggregates/${code}`),
  },
  indicators: {
    list: () => getJson<Indicator[]>('/indicators'),
    operands: () => getJson<IndicatorOperand[]>('/indicators/operands'),
    preview: (body: { expression: string; grain?: string[]; consolidation_id: number }) =>
      postJsonRaw<IndicatorPreview>('/indicators/preview', body),
    create: (body: {
      code: string;
      libelle?: string;
      expression: string;
      grain?: string[];
      format?: string;
    }) => postJsonRaw<Indicator>('/indicators', body),
    update: (
      code: string,
      body: { libelle?: string; expression: string; grain?: string[]; format?: string },
    ) => putJson<Indicator>(`/indicators/${code}`, { code, ...body }),
    remove: (code: string) => deleteJson<{ deleted: string }>(`/indicators/${code}`),
  },
  // Contrôles de données
  controls: {
    list: () => getJson<Control[]>('/controls'),
    get: (code: string) => getJson<Control>(`/controls/${code}`),
    create: (body: { code: string; libelle?: string; definition: ControlDefinition }) =>
      postJsonRaw<Control>('/controls', body),
    update: (code: string, body: { libelle?: string; definition: ControlDefinition }) =>
      putJson<Control>(`/controls/${code}`, { code, ...body }),
    remove: (code: string) => deleteJson<{ deleted: string }>(`/controls/${code}`),
    rename: (code: string, newCode: string) =>
      postJsonRaw<{ renamed: { old: string; new: string } }>(
        `/controls/${code}/rename`,
        { new: newCode },
      ),
    run: (code: string, params: { consolidation_id?: number; phase?: string; entry_period?: string }) =>
      postJsonRaw<ControlReport>(`/controls/${code}/run`, params),
    operands: () => getJson<ControlOperand[]>('/controls/operands'),
  },
  controlSets: {
    list: () => getJson<ControlSet[]>('/control-sets'),
    get: (code: string) => getJson<ControlSet>(`/control-sets/${code}`),
    create: (body: { code: string; libelle?: string; controls: { code: string; ord?: number }[] }) =>
      postJsonRaw<ControlSet>('/control-sets', body),
    update: (code: string, body: { libelle?: string; controls: { code: string; ord?: number }[] }) =>
      putJson<ControlSet>(`/control-sets/${code}`, { code, ...body }),
    remove: (code: string) => deleteJson<{ deleted: string }>(`/control-sets/${code}`),
    rename: (code: string, newCode: string) =>
      postJsonRaw<{ renamed: { old: string; new: string } }>(
        `/control-sets/${code}/rename`,
        { new: newCode },
      ),
    run: (code: string, params: { consolidation_id?: number; phase?: string; entry_period?: string }) =>
      postJsonRaw<ControlSetReport>(`/control-sets/${code}/run`, params),
    results: (code: string) => getJson<ControlSetReport>(`/control-sets/${code}/results`),
  },
  // Graphe des références (source de vérité serveur), pour les dropdowns
  // contextuels — remplace les miroirs codés en dur côté front.
  references: () => getJson<ReferenceInfo[]>('/meta/references'),
  // Enums natifs (CHECK du DDL des master data, ex : account.classe).
  // Source de vérité pour le mode `attr` de `SelectionCond`.
  nativeEnums: () => getJson<NativeEnum[]>('/meta/native-enums'),
  // Rapport « santé des données » : orphelins sur tout le graphe de références.
  dataHealth: () => getJson<DataHealthReport>('/meta/health'),
  dimensions: {
    list: () => getJson<DimensionInfo[]>('/meta/dimensions'),
    create: (body: { name: string; label: string }) =>
      postJsonRaw<DimensionInfo>('/meta/dimensions', body),
    remove: (name: string) =>
      deleteJson<{ deleted: number }>(`/meta/dimensions/${name}`),
  },
  // Caractéristiques N1/N2 : regroupement d'une dimension par une caractéristique
  // dont les attributs pointent vers d'autres dimensions (cf. characteristics.rs).
  characteristics: {
    list: () => getJson<Characteristic[]>('/meta/characteristics'),
    create: (body: { code: string; libelle: string; base_dimension: string }) =>
      postJsonRaw<{ code: string }>('/meta/characteristics', body),
    update: (code: string, body: { libelle: string }) =>
      putJson<{ code: string; libelle: string }>(`/meta/characteristics/${code}`, body),
    remove: (code: string) =>
      deleteJson<{ deleted: string }>(`/meta/characteristics/${code}`),
    addAttribute: (
      code: string,
      body: { name: string; libelle: string; target_dimension: string },
    ) => postJsonRaw<unknown>(`/meta/characteristics/${code}/attributes`, body),
    removeAttribute: (code: string, name: string) =>
      deleteJson<unknown>(`/meta/characteristics/${code}/attributes/${name}`),
    listValues: (code: string) =>
      getJson<Record<string, unknown>[]>(`/meta/characteristics/${code}/values`),
    createValue: (code: string, row: Record<string, unknown>) =>
      postJsonRaw<unknown>(`/meta/characteristics/${code}/values`, row),
    updateValue: (code: string, value: string, row: Record<string, unknown>) =>
      putJson<unknown>(`/meta/characteristics/${code}/values/${value}`, row),
    removeValue: (code: string, value: string) =>
      deleteJson<unknown>(`/meta/characteristics/${code}/values/${value}`),
    renameValue: (code: string, value: string, newCode: string) =>
      postJsonRaw<{ renamed: { old: string; new: string } }>(
        `/meta/characteristics/${code}/values/${value}/rename`,
        { new_code: newCode },
      ),
    assign: (code: string, body: { member: string; value: string | null }) =>
      putJson<unknown>(`/meta/characteristics/${code}/assign`, body),
    rename: (code: string, newCode: string) =>
      postJsonRaw<{ renamed: { old: string; new: string } }>(
        `/meta/characteristics/${code}/rename`,
        { new_code: newCode },
      ),
  },
  // Listes de valeurs (référentiels) : nomenclatures code/libellé autonomes,
  // réutilisables comme cible d'un attribut N2, mais qui ne sont pas des
  // dimensions. Cf. value_lists.rs.
  valueLists: {
    list: () => getJson<ValueList[]>('/meta/value-lists'),
    create: (body: { code: string; libelle: string }) =>
      postJsonRaw<{ code: string }>('/meta/value-lists', body),
    update: (code: string, body: { libelle: string }) =>
      putJson<{ code: string; libelle: string }>(`/meta/value-lists/${code}`, body),
    remove: (code: string) =>
      deleteJson<{ deleted: string }>(`/meta/value-lists/${code}`),
    listValues: (code: string) =>
      getJson<Record<string, unknown>[]>(`/meta/value-lists/${code}/values`),
    createValue: (code: string, row: { code: string; libelle?: string }) =>
      postJsonRaw<unknown>(`/meta/value-lists/${code}/values`, row),
    updateValue: (code: string, value: string, row: { libelle?: string }) =>
      putJson<unknown>(`/meta/value-lists/${code}/values/${value}`, row),
    removeValue: (code: string, value: string) =>
      deleteJson<unknown>(`/meta/value-lists/${code}/values/${value}`),
    renameValue: (code: string, value: string, newCode: string) =>
      postJsonRaw<{ renamed: { old: string; new: string } }>(
        `/meta/value-lists/${code}/values/${value}/rename`,
        { new_code: newCode },
      ),
    rename: (code: string, newCode: string) =>
      postJsonRaw<{ renamed: { old: string; new: string } }>(
        `/meta/value-lists/${code}/rename`,
        { new_code: newCode },
      ),
  },
  // Références directes (patron B) : colonne sur une dimension hôte pointant vers
  // une dimension cible (y compris elle-même). Cf. custom_references.rs.
  customReferences: {
    list: () => getJson<CustomReference[]>('/meta/references-custom'),
    create: (body: { host_dimension: string; column: string; target_dimension: string }) =>
      postJsonRaw<{ host_dimension: string; column: string }>('/meta/references-custom', body),
    remove: (host: string, column: string) =>
      deleteJson<{ deleted: string }>(`/meta/references-custom/${host}/${column}`),
    assign: (host: string, column: string, body: { member: string; value: string | null }) =>
      putJson<unknown>(`/meta/references-custom/${host}/${column}/assign`, body),
  },
};
