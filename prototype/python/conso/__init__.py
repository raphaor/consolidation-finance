"""Moteur de consolidation financière par les flux — prototype Python/DuckDB.

Ce package implémente le pipeline de consolidation en 4 étapes
(agrégation → reclassification → conversion → consolidation) sur une base
DuckDB embarquée. La structure est pensée pour un futur portage en Rust :
chaque étape est une fonction isolée qui lit un niveau de stockage et produit
le suivant via du SQL déclaratif.
"""

__all__ = ["schema", "seed", "pipeline", "validate", "report"]
