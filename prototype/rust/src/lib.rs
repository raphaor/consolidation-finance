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
//! **Identité fondamentale** : `F99 = F00 + F01 + F20 + F80 + F81 + F98`

pub mod import;
pub mod loader;
pub mod masterdata;
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
