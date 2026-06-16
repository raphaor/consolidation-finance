//! Montants financiers en précision décimale exacte (`rust_decimal::Decimal`).
//!
//! Le moteur manipule des montants d'écritures stockés en base sous le type
//! DuckDB `DECIMAL(18,2)`. Côté Rust, on utilise `rust_decimal::Decimal` plutôt
//! que `f64` pour garantir une précision exacte (pas d'erreur d'arrondi
//! flottant — critique pour de la finance).
//!
//! # Lecture / écriture DuckDB
//!
//! duckdb-rs 1.1 expose les `DECIMAL` via `Value::Decimal(rust_decimal::Decimal)`
//! mais n'implémente pas `FromSql` directement pour `Decimal`. On passe donc
//! par un wrapper [`Money`] qui implémente `FromSql` (lecture) et `ToSql`
//! (écriture via `Value::Decimal`).

use duckdb::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, Value, ValueRef};
use rust_decimal::Decimal;
use rust_decimal::prelude::*;

/// Type monnaie : wrapper autour de `rust_decimal::Decimal` compatible DuckDB.
///
/// Utilisé comme cible de `row.get::<_, Money>(i)?` dans les `query_map` /
/// `query_row`. Pour récupérer le `Decimal` interne : `money.0` ou
/// `money.into_decimal()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Money(pub Decimal);

impl Money {
    #[inline]
    pub fn into_decimal(self) -> Decimal {
        self.0
    }
}

impl From<Decimal> for Money {
    #[inline]
    fn from(d: Decimal) -> Self {
        Self(d)
    }
}

impl From<Money> for Decimal {
    #[inline]
    fn from(m: Money) -> Self {
        m.0
    }
}

impl From<i64> for Money {
    #[inline]
    fn from(i: i64) -> Self {
        Self(Decimal::from(i))
    }
}

/// Lecture : `Value::Decimal(d)` → `Money(d)`. On accepte aussi les entiers
/// (sécurité si une colonne sort en BIGINT au lieu de DECIMAL).
impl FromSql for Money {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value {
            ValueRef::Decimal(d) => Ok(Money(d)),
            ValueRef::BigInt(i) => Ok(Money(Decimal::from(i))),
            ValueRef::Int(i) => Ok(Money(Decimal::from(i))),
            ValueRef::HugeInt(i) => {
                Decimal::from_i128(i).map(Money).ok_or(FromSqlError::OutOfRange(i))
            }
            _ => Err(FromSqlError::InvalidType),
        }
    }
}

/// Écriture : on émet un `Value::Decimal(d)` que duckdb sait binder sur une
/// colonne `DECIMAL(18,2)`.
impl ToSql for Money {
    fn to_sql(&self) -> duckdb::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Owned(Value::Decimal(self.0)))
    }
}
