//! Moteur de consolidation financière par les flux.
//!
//! Portage Rust du prototype Python (`prototype/python/conso/`).
//!
//! # Architecture
//!
//! ```text
//! staging (stg_entry)
//!     │  A. agrégation (+ reconstruction des clôtures)
//!     ▼
//! fact_entry [corporate]       — devise fonctionnelle
//!     │  C. conversion multi-devises (+ écarts F80/F81)
//!     ▼
//! fact_entry [converted]       — devise de présentation
//!     │  D. consolidation (méthodes × pct_integration)
//!     ▼
//! fact_entry [consolidated]    — devise de présentation
//! ```
//!
//! L'étape B (reclassification de périmètre) et le niveau `reclassified` ont été
//! supprimés : le périmètre (F00→F01, miroir F98) passe par des règles au niveau
//! corporate (cf. docs/A_NOUVEAU.md §4).
//!
//! # Modèle de flux (cf. docs/FLUX_CONSO.md)
//!
//! | Code | Libellé                | Taux conv.  | Flux écart |
//! |------|------------------------|-------------|------------|
//! | F00  | Ouverture              | close_n1    | F80        |
//! | F01  | Entrée périmètre       | close_n1    | F80        |
//! | F20  | Variation              | avg         | F81        |
//! | F80  | Écart conv. ouverture  | close_n     | —          |
//! | F81  | Écart conv. variation  | close_n     | —          |
//! | F98  | Sortie périmètre       | close_n     | —          |
//! | F99  | Clôture                | close_n     | —          |
//!
//! **Reconstruction des clôtures** : un flux auto-référentiel
//! (`flux_de_report(C) = C`) est une clôture reconstruite comme
//! `C = Σ(X | flux_de_report(X) = C et X ≠ C)` (cf. `pipeline::materialize_closures`).
//! Aujourd'hui seule F99 est une clôture ; la logique est générique et pilotée
//! par `dim_flow.flux_de_report`.

pub mod characteristics;
pub mod coefficients;
pub mod controls;
pub mod custom_references;
pub mod entries;
pub mod formula;
pub mod indicators;
pub mod json_migration;
pub mod dimensions;
pub mod export;
pub mod import;
pub mod masterdata;
pub mod money;
pub mod pipeline;
pub mod references;
pub mod report;
pub mod reports;
pub mod resolve;
pub mod rules;
pub mod schema;
pub mod seed;
pub mod state;
pub mod surrogate;
pub mod validate;
pub mod value_lists;

// Ré-exports pour faciliter l'usage depuis le binaire.
pub use pipeline::{run_pipeline, run_pipeline_with_hook, ConvertParams};
pub use rules::run_ruleset;
pub use schema::create_schema;
pub use seed::{seed_all, seed_demo_controls};
pub use state::AppState;
