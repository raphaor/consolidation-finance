//! Moteur de consolidation financière par les flux.
//!
//! Portage Rust du prototype Python (`prototype/python/conso/`).
//!
//! # Architecture
//!
//! ```text
//! staging (stg_entry)
//!     │  A. agrégation
//!     ▼
//! fact_entry [corporate]       — devise fonctionnelle
//!     │  B. reclassification de périmètre
//!     ▼
//! fact_entry [reclassified]    — devise fonctionnelle
//!     │  C. conversion multi-devises (+ écarts F80/F81)
//!     ▼
//! fact_entry [converted]       — devise de présentation
//!     │  D. consolidation (méthodes × pct_integration)
//!     ▼
//! fact_entry [consolidated]    — devise de présentation
//! ```
//!
//! # Modèle de flux (cf. docs/FLUX_CONSO.md)
//!
//! | Code | Libellé                | Taux conv.  | Flux écart |
//! |------|------------------------|-------------|------------|
//! | F00  | Ouverture              | close_n1    | F80        |
//! | F01  | Entrée périmètre       | close_n1    | F80        |
//! | F20  | Variation              | avg         | F81        |
//! | F80  | Écart conv. ouverture  | terminal    | —          |
//! | F81  | Écart conv. variation  | terminal    | —          |
//! | F98  | Sortie périmètre       | terminal    | —          |
//! | F99  | Clôture                | close_n     | —          |
//!
//! **Reconstruction des clôtures** : un flux auto-référentiel
//! (`flux_de_report(C) = C`) est une clôture reconstruite comme
//! `C = Σ(X | flux_de_report(X) = C et X ≠ C)` (cf. `pipeline::materialize_closures`).
//! Aujourd'hui seule F99 est une clôture ; la logique est générique et pilotée
//! par `dim_flow.flux_de_report`.

pub mod import;
pub mod loader;
pub mod masterdata;
pub mod money;
pub mod pipeline;
pub mod report;
pub mod schema;
pub mod seed;
pub mod state;
pub mod validate;

// Ré-exports pour faciliter l'usage depuis le binaire.
pub use loader::load_all;
pub use pipeline::{run_pipeline, ConvertParams};
pub use schema::create_schema;
pub use seed::seed_all;
pub use state::AppState;
