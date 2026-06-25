//! Moteur de formules — langage type Excel pour les coefficients (volet 1) et,
//! à terme, les indicateurs/KPI (volet 2). Spec : `docs/FORMULES.md`.
//!
//! **Pur** (aucune dépendance DB). Trois étages :
//!
//! 1. [`parse`] : texte → AST (lexer + parser à descente récursive).
//! 2. [`compile`] : AST + [`OperandResolver`] → `(expression SQL, CoeffJoins)`.
//!    C'est le point d'insertion dans le moteur de règles, à la place de
//!    l'ancien `coefficient_expr` (cf. `rules::resolve_coefficient`).
//! 3. [`evaluate`] : AST + valeurs d'exemple → `f64`. Interpréteur servant la
//!    **preview live** de l'éditeur (sans toucher la base).
//!
//! # Langage (cf. `docs/FORMULES.md` §2)
//!
//! - Opérateurs : `+ - * /` (et leurs variantes unicode `× ÷ −`), parenthèses,
//!   comparateurs `> < >= <= = <>` (dans un `IF`).
//! - Fonctions : `MIN`, `MAX`, `ABS`, `ROUND`, `IF`, `SAFE_DIV`.
//! - Références entre `[ … ]`, résolues contre le catalogue d'opérandes du
//!   contexte (périmètre pour les coefficients).
//! - Séparateur d'arguments : `;` (convention Excel francophone).
//!
//! # Sécurité SQL
//!
//! Les noms d'opérandes sont validés par le résolveur contre une whitelist
//! (colonnes de `sat_perimeter` + perspectives fixes) ; seules les **constantes
//! numériques** sont émises comme littéraux. Aucun identifiant issu du texte
//! utilisateur n'est interpolé brut (cf. [`PerimeterResolver`]).

use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
//  Jointures de périmètre (accumulateur du contexte coefficient)
// ─────────────────────────────────────────────────────────────────────────────

/// Perspectives de `sat_perimeter` qu'une expression de coefficient lit, pour
/// que `rules::exec_operation` ajoute les JOINs correspondants :
/// - `p_ent` / `p_part` : entité / partenaire à la **période courante** ;
/// - `p_ent_n1` / `p_part_n1` : idem à la période **N-1** (via à-nouveau).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CoeffJoins {
    pub p_ent: bool,
    pub p_part: bool,
    pub p_ent_n1: bool,
    pub p_part_n1: bool,
}

impl CoeffJoins {
    fn merge(self, o: CoeffJoins) -> CoeffJoins {
        CoeffJoins {
            p_ent: self.p_ent || o.p_ent,
            p_part: self.p_part || o.p_part,
            p_ent_n1: self.p_ent_n1 || o.p_ent_n1,
            p_part_n1: self.p_part_n1 || o.p_part_n1,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Résolveur d'opérandes (abstraction — cf. F5 : garder le compilateur agnostique)
// ─────────────────────────────────────────────────────────────────────────────

/// Opérande résolu : expression SQL + jointures de périmètre requises.
pub struct Resolved {
    pub sql: String,
    pub joins: CoeffJoins,
}

/// Fournit le compilateur en opérandes. Pour les coefficients,
/// [`PerimeterResolver`] mappe un nom `champ.perspective` vers une lecture de
/// `sat_perimeter`. Le volet 2 (indicateurs) fournira un autre résolveur sans
/// toucher au langage.
pub trait OperandResolver {
    fn resolve(&self, name: &str) -> Result<Resolved, String>;
}

/// Résolveur **périmètre** (contexte coefficient). Un opérande s'écrit
/// `<champ>.<perspective>` où `<champ>` est une colonne numérique de
/// `sat_perimeter` (whitelist injectée) et `<perspective>` ∈
/// `entity` / `partner` / `entity_n1` / `partner_n1`.
///
/// Émet `COALESCE(p_<alias>.<champ>, 0)` — **défaut uniforme 0** (décision F3,
/// `docs/FORMULES.md` §3.2) — et lève le flag de JOIN correspondant.
pub struct PerimeterResolver {
    /// Colonnes numériques autorisées de `sat_perimeter` (depuis information_schema).
    pub fields: Vec<String>,
}

impl PerimeterResolver {
    pub fn new(fields: Vec<String>) -> Self {
        Self { fields }
    }
}

impl OperandResolver for PerimeterResolver {
    fn resolve(&self, name: &str) -> Result<Resolved, String> {
        let (field, perspective) = name.split_once('.').ok_or_else(|| {
            format!(
                "opérande '{name}' invalide : attendu « champ.perspective » \
                 (ex. pct_integration.entity)"
            )
        })?;
        if !self.fields.iter().any(|f| f == field) {
            return Err(format!(
                "opérande '{name}' : champ de périmètre inconnu '{field}' \
                 (disponibles : {:?})",
                self.fields
            ));
        }
        // perspective → alias SQL + flag de JOIN. Liste fermée (sécurité).
        let (alias, joins) = match perspective {
            "entity" => (
                "p_ent",
                CoeffJoins {
                    p_ent: true,
                    ..Default::default()
                },
            ),
            "partner" => (
                "p_part",
                CoeffJoins {
                    p_part: true,
                    ..Default::default()
                },
            ),
            "entity_n1" => (
                "p_ent_n1",
                CoeffJoins {
                    p_ent_n1: true,
                    ..Default::default()
                },
            ),
            "partner_n1" => (
                "p_part_n1",
                CoeffJoins {
                    p_part_n1: true,
                    ..Default::default()
                },
            ),
            other => {
                return Err(format!(
                    "opérande '{name}' : perspective inconnue '{other}' \
                     (attendu entity / partner / entity_n1 / partner_n1)"
                ))
            }
        };
        // `field` est whitelisté, `alias` est une constante → pas d'injection.
        Ok(Resolved {
            sql: format!("COALESCE({alias}.{field}, 0)"),
            joins,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Lexer
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Num(f64),
    Ref(String),   // contenu entre [ ]
    Ident(String), // nom de fonction
    Plus,
    Minus,
    Star,
    Slash,
    LParen,
    RParen,
    Semicolon,
    Gt,
    Lt,
    Ge,
    Le,
    Eq,
    Ne,
}

fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' | '\n' | '\r' => {
                i += 1;
            }
            // Opérateurs (ASCII + variantes unicode).
            '+' => {
                out.push(Token::Plus);
                i += 1;
            }
            '-' | '−' => {
                out.push(Token::Minus);
                i += 1;
            }
            '*' | '×' => {
                out.push(Token::Star);
                i += 1;
            }
            '/' | '÷' => {
                out.push(Token::Slash);
                i += 1;
            }
            '(' => {
                out.push(Token::LParen);
                i += 1;
            }
            ')' => {
                out.push(Token::RParen);
                i += 1;
            }
            ';' => {
                out.push(Token::Semicolon);
                i += 1;
            }
            // Erreur fréquente (réflexe Excel anglophone) : on guide vers ';'.
            ',' => {
                return Err(
                    "',' n'est pas le séparateur d'arguments : utilisez ';' \
                     (ex. SAFE_DIV([a]; [b]))"
                        .to_string(),
                );
            }
            '>' => {
                if chars.get(i + 1) == Some(&'=') {
                    out.push(Token::Ge);
                    i += 2;
                } else {
                    out.push(Token::Gt);
                    i += 1;
                }
            }
            '<' => match chars.get(i + 1) {
                Some('=') => {
                    out.push(Token::Le);
                    i += 2;
                }
                Some('>') => {
                    out.push(Token::Ne);
                    i += 2;
                }
                _ => {
                    out.push(Token::Lt);
                    i += 1;
                }
            },
            '=' => {
                out.push(Token::Eq);
                i += 1;
            }
            '!' if chars.get(i + 1) == Some(&'=') => {
                out.push(Token::Ne);
                i += 2;
            }
            '[' => {
                let mut j = i + 1;
                let mut name = String::new();
                while j < chars.len() && chars[j] != ']' {
                    name.push(chars[j]);
                    j += 1;
                }
                if j >= chars.len() {
                    return Err("référence '[' non fermée (']' manquant)".to_string());
                }
                let trimmed = name.trim().to_string();
                if trimmed.is_empty() {
                    return Err("référence '[]' vide".to_string());
                }
                out.push(Token::Ref(trimmed));
                i = j + 1; // saute le ']'
            }
            c if c.is_ascii_digit() || c == '.' => {
                let mut j = i;
                let mut seen_dot = false;
                let mut s = String::new();
                while j < chars.len() && (chars[j].is_ascii_digit() || chars[j] == '.') {
                    if chars[j] == '.' {
                        if seen_dot {
                            return Err("nombre mal formé (deux points décimaux)".to_string());
                        }
                        seen_dot = true;
                    }
                    s.push(chars[j]);
                    j += 1;
                }
                let v: f64 = s
                    .parse()
                    .map_err(|_| format!("nombre invalide : '{s}'"))?;
                out.push(Token::Num(v));
                i = j;
            }
            c if c.is_alphabetic() || c == '_' => {
                let mut j = i;
                let mut s = String::new();
                while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                    s.push(chars[j]);
                    j += 1;
                }
                out.push(Token::Ident(s));
                i = j;
            }
            other => return Err(format!("caractère inattendu : '{other}'")),
        }
    }
    Ok(out)
}

// ─────────────────────────────────────────────────────────────────────────────
//  AST + parser
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Cmp {
    Gt,
    Lt,
    Ge,
    Le,
    Eq,
    Ne,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Func {
    Min,
    Max,
    Abs,
    Round,
    If,
    SafeDiv,
}

impl Func {
    fn from_name(name: &str) -> Option<Func> {
        match name.to_ascii_uppercase().as_str() {
            "MIN" => Some(Func::Min),
            "MAX" => Some(Func::Max),
            "ABS" => Some(Func::Abs),
            "ROUND" => Some(Func::Round),
            "IF" => Some(Func::If),
            "SAFE_DIV" => Some(Func::SafeDiv),
            _ => None,
        }
    }

    /// Arités acceptées : (min, max). `max = None` → variadique.
    fn arity(self) -> (usize, Option<usize>) {
        match self {
            Func::Min | Func::Max => (2, None),
            Func::Abs => (1, Some(1)),
            Func::Round => (2, Some(2)),
            Func::If => (3, Some(3)),
            Func::SafeDiv => (2, Some(2)),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Expr {
    Num(f64),
    Ref(String),
    Neg(Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    Cmp(Cmp, Box<Expr>, Box<Expr>),
    Call(Func, Vec<Expr>),
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.toks.get(self.pos)
    }
    fn next(&mut self) -> Option<Token> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }
    fn expect(&mut self, t: &Token) -> Result<(), String> {
        if self.peek() == Some(t) {
            self.pos += 1;
            Ok(())
        } else {
            Err(format!("attendu {t:?}, trouvé {:?}", self.peek()))
        }
    }

    // expr := comparison
    fn parse_expr(&mut self) -> Result<Expr, String> {
        let left = self.parse_add()?;
        let cmp = match self.peek() {
            Some(Token::Gt) => Cmp::Gt,
            Some(Token::Lt) => Cmp::Lt,
            Some(Token::Ge) => Cmp::Ge,
            Some(Token::Le) => Cmp::Le,
            Some(Token::Eq) => Cmp::Eq,
            Some(Token::Ne) => Cmp::Ne,
            _ => return Ok(left),
        };
        self.pos += 1;
        let right = self.parse_add()?;
        Ok(Expr::Cmp(cmp, Box::new(left), Box::new(right)))
    }

    fn parse_add(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_mul()?;
        loop {
            let op = match self.peek() {
                Some(Token::Plus) => BinOp::Add,
                Some(Token::Minus) => BinOp::Sub,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_mul()?;
            left = Expr::Bin(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_mul(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Some(Token::Star) => BinOp::Mul,
                Some(Token::Slash) => BinOp::Div,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_unary()?;
            left = Expr::Bin(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        match self.peek() {
            Some(Token::Minus) => {
                self.pos += 1;
                Ok(Expr::Neg(Box::new(self.parse_unary()?)))
            }
            Some(Token::Plus) => {
                self.pos += 1;
                self.parse_unary()
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        match self.next() {
            Some(Token::Num(n)) => Ok(Expr::Num(n)),
            Some(Token::Ref(name)) => Ok(Expr::Ref(name)),
            Some(Token::LParen) => {
                let e = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                Ok(e)
            }
            Some(Token::Ident(name)) => {
                let func = Func::from_name(&name)
                    .ok_or_else(|| format!("fonction inconnue : '{name}'"))?;
                self.expect(&Token::LParen)?;
                let mut args = Vec::new();
                if self.peek() != Some(&Token::RParen) {
                    args.push(self.parse_expr()?);
                    while self.peek() == Some(&Token::Semicolon) {
                        self.pos += 1;
                        args.push(self.parse_expr()?);
                    }
                }
                self.expect(&Token::RParen)?;
                let (lo, hi) = func.arity();
                if args.len() < lo || hi.map(|h| args.len() > h).unwrap_or(false) {
                    return Err(format!(
                        "{name} : {} argument(s), attendu {}",
                        args.len(),
                        match hi {
                            Some(h) if h == lo => format!("{lo}"),
                            Some(h) => format!("entre {lo} et {h}"),
                            None => format!("au moins {lo}"),
                        }
                    ));
                }
                Ok(Expr::Call(func, args))
            }
            other => Err(format!("expression attendue, trouvé {other:?}")),
        }
    }
}

fn parse(input: &str) -> Result<Expr, String> {
    let toks = tokenize(input)?;
    if toks.is_empty() {
        return Err("formule vide".to_string());
    }
    let mut p = Parser { toks, pos: 0 };
    let e = p.parse_expr()?;
    if p.pos != p.toks.len() {
        return Err(format!(
            "tokens en trop à partir de {:?}",
            p.toks.get(p.pos)
        ));
    }
    Ok(e)
}

// ─────────────────────────────────────────────────────────────────────────────
//  Compilation → SQL
// ─────────────────────────────────────────────────────────────────────────────

/// Formate un littéral numérique en SQL (point décimal, pas de séparateur).
/// Aligné sur `rules::format_float` pour la cohérence des montants.
fn format_num(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{v:.1}")
    } else {
        format!("{v}")
    }
}

fn compile_expr(e: &Expr, r: &dyn OperandResolver, joins: &mut CoeffJoins) -> Result<String, String> {
    Ok(match e {
        Expr::Num(n) => format_num(*n),
        Expr::Ref(name) => {
            let res = r.resolve(name)?;
            *joins = joins.merge(res.joins);
            res.sql
        }
        Expr::Neg(a) => format!("(-({}))", compile_expr(a, r, joins)?),
        Expr::Bin(op, a, b) => {
            let sa = compile_expr(a, r, joins)?;
            let sb = compile_expr(b, r, joins)?;
            let sym = match op {
                BinOp::Add => "+",
                BinOp::Sub => "-",
                BinOp::Mul => "*",
                BinOp::Div => "/",
            };
            format!("({sa} {sym} {sb})")
        }
        Expr::Cmp(cmp, a, b) => {
            let sa = compile_expr(a, r, joins)?;
            let sb = compile_expr(b, r, joins)?;
            let sym = match cmp {
                Cmp::Gt => ">",
                Cmp::Lt => "<",
                Cmp::Ge => ">=",
                Cmp::Le => "<=",
                Cmp::Eq => "=",
                Cmp::Ne => "<>",
            };
            format!("({sa} {sym} {sb})")
        }
        Expr::Call(func, args) => {
            let c: Vec<String> = args
                .iter()
                .map(|a| compile_expr(a, r, joins))
                .collect::<Result<_, _>>()?;
            match func {
                Func::Abs => format!("ABS({})", c[0]),
                Func::Round => format!("ROUND({}, {})", c[0], c[1]),
                // SAFE_DIV : 0 si dénominateur nul (cf. F3 — protection explicite).
                Func::SafeDiv => {
                    format!("CASE WHEN ({}) = 0 THEN 0 ELSE ({}) / ({}) END", c[1], c[0], c[1])
                }
                Func::If => format!("CASE WHEN ({}) THEN ({}) ELSE ({}) END", c[0], c[1], c[2]),
                // MIN/MAX compilés en CASE imbriqués (pas LEAST/GREATEST qui
                // ignorent les NULL sous DuckDB — cf. docs/FORMULES.md §2.2).
                Func::Min => fold_minmax(&c, "<="),
                Func::Max => fold_minmax(&c, ">="),
            }
        }
    })
}

/// Replie une liste d'expressions en `CASE` imbriqués pour MIN/MAX.
/// `cmp` = `<=` pour MIN (garde le plus petit), `>=` pour MAX.
fn fold_minmax(args: &[String], cmp: &str) -> String {
    let mut acc = args[0].clone();
    for next in &args[1..] {
        acc = format!("CASE WHEN ({acc}) {cmp} ({next}) THEN ({acc}) ELSE ({next}) END");
    }
    acc
}

/// Compile une formule en `(expression SQL, jointures de périmètre)`.
pub fn compile(input: &str, resolver: &dyn OperandResolver) -> Result<(String, CoeffJoins), String> {
    let ast = parse(input)?;
    let mut joins = CoeffJoins::default();
    let sql = compile_expr(&ast, resolver, &mut joins)?;
    Ok((sql, joins))
}

// ─────────────────────────────────────────────────────────────────────────────
//  Interpréteur (preview live)
// ─────────────────────────────────────────────────────────────────────────────

fn eval_expr(e: &Expr, samples: &HashMap<String, f64>) -> Result<f64, String> {
    Ok(match e {
        Expr::Num(n) => *n,
        // Opérande absent → 0 (défaut uniforme F3).
        Expr::Ref(name) => *samples.get(name).unwrap_or(&0.0),
        Expr::Neg(a) => -eval_expr(a, samples)?,
        Expr::Bin(op, a, b) => {
            let va = eval_expr(a, samples)?;
            let vb = eval_expr(b, samples)?;
            match op {
                BinOp::Add => va + vb,
                BinOp::Sub => va - vb,
                BinOp::Mul => va * vb,
                BinOp::Div => va / vb, // pas de garde-fou (F3) ; non-fini détecté en sortie
            }
        }
        Expr::Cmp(cmp, a, b) => {
            let va = eval_expr(a, samples)?;
            let vb = eval_expr(b, samples)?;
            let r = match cmp {
                Cmp::Gt => va > vb,
                Cmp::Lt => va < vb,
                Cmp::Ge => va >= vb,
                Cmp::Le => va <= vb,
                Cmp::Eq => va == vb,
                Cmp::Ne => va != vb,
            };
            if r {
                1.0
            } else {
                0.0
            }
        }
        Expr::Call(func, args) => {
            let v: Vec<f64> = args
                .iter()
                .map(|a| eval_expr(a, samples))
                .collect::<Result<_, _>>()?;
            match func {
                Func::Abs => v[0].abs(),
                Func::Round => {
                    let p = 10f64.powi(v[1] as i32);
                    (v[0] * p).round() / p
                }
                Func::SafeDiv => {
                    if v[1] == 0.0 {
                        0.0
                    } else {
                        v[0] / v[1]
                    }
                }
                Func::If => {
                    if v[0] != 0.0 {
                        v[1]
                    } else {
                        v[2]
                    }
                }
                Func::Min => v.iter().cloned().fold(f64::INFINITY, f64::min),
                Func::Max => v.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            }
        }
    })
}

/// Évalue une formule sur des valeurs d'exemple (preview). Opérande absent → 0.
pub fn evaluate(input: &str, samples: &HashMap<String, f64>) -> Result<f64, String> {
    let ast = parse(input)?;
    let v = eval_expr(&ast, samples)?;
    if !v.is_finite() {
        return Err("résultat non fini (division par zéro ? envisagez SAFE_DIV)".to_string());
    }
    Ok(v)
}

/// Liste les noms d'opérandes (`[ … ]`) référencés par une formule. Utile pour
/// pré-remplir les valeurs d'exemple de la preview et pour l'éditeur.
pub fn operands(input: &str) -> Result<Vec<String>, String> {
    let ast = parse(input)?;
    let mut out = Vec::new();
    collect_refs(&ast, &mut out);
    Ok(out)
}

fn collect_refs(e: &Expr, out: &mut Vec<String>) {
    match e {
        Expr::Num(_) => {}
        Expr::Ref(name) => {
            if !out.contains(name) {
                out.push(name.clone());
            }
        }
        Expr::Neg(a) => collect_refs(a, out),
        Expr::Bin(_, a, b) | Expr::Cmp(_, a, b) => {
            collect_refs(a, out);
            collect_refs(b, out);
        }
        Expr::Call(_, args) => {
            for a in args {
                collect_refs(a, out);
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn perim() -> PerimeterResolver {
        PerimeterResolver::new(vec!["pct_integration".into(), "pct_interet".into()])
    }

    #[test]
    fn parse_nombre_et_arithmetique() {
        assert!(parse("1 + 2 * 3").is_ok());
        assert!(parse("(1 + 2) * 3").is_ok());
        assert!(parse("-[pct_integration.entity]").is_ok());
    }

    #[test]
    fn parenthese_non_fermee_rejetee() {
        assert!(parse("(1 + 2").is_err());
        assert!(parse("[pct_integration.entity").is_err());
    }

    #[test]
    fn fonction_inconnue_rejetee() {
        assert!(parse("FOO(1; 2)").is_err());
    }

    #[test]
    fn arite_verifiee() {
        assert!(parse("ABS(1; 2)").is_err());
        assert!(parse("ROUND(1)").is_err());
        assert!(parse("IF(1 > 0; 2)").is_err());
        assert!(parse("SAFE_DIV(1)").is_err());
        assert!(parse("MIN(1)").is_err());
        assert!(parse("MIN(1; 2; 3)").is_ok());
    }

    #[test]
    fn virgule_guide_vers_point_virgule() {
        // Réflexe Excel anglophone : la virgule doit produire un message clair.
        let err = parse("SAFE_DIV(1, 2)").unwrap_err();
        assert!(err.contains("';'"), "message doit pointer vers ';' : {err}");
    }

    #[test]
    fn compile_operande_perimetre_et_joins() {
        let (sql, j) = compile("[pct_integration.entity]", &perim()).unwrap();
        assert_eq!(sql, "COALESCE(p_ent.pct_integration, 0)");
        assert_eq!(
            j,
            CoeffJoins {
                p_ent: true,
                ..Default::default()
            }
        );
    }

    #[test]
    fn compile_elim_ic_corp_n_joins_entity_et_partner() {
        let (sql, j) = compile(
            "MIN(1; SAFE_DIV([pct_integration.partner]; [pct_integration.entity]))",
            &perim(),
        )
        .unwrap();
        assert!(sql.contains("p_part.pct_integration"));
        assert!(sql.contains("p_ent.pct_integration"));
        assert!(sql.contains("CASE WHEN")); // MIN + SAFE_DIV compilés en CASE
        assert!(j.p_ent && j.p_part);
        assert!(!j.p_ent_n1 && !j.p_part_n1);
    }

    #[test]
    fn compile_var_joins_n1() {
        let (_, j) = compile(
            "MIN(1; SAFE_DIV([pct_integration.partner]; [pct_integration.entity])) \
             - MIN(1; SAFE_DIV([pct_integration.partner_n1]; [pct_integration.entity_n1]))",
            &perim(),
        )
        .unwrap();
        assert!(j.p_ent && j.p_part && j.p_ent_n1 && j.p_part_n1);
    }

    #[test]
    fn champ_inconnu_rejete() {
        let err = compile("[methode.entity]", &perim()).unwrap_err();
        assert!(err.contains("inconnu"));
    }

    #[test]
    fn perspective_inconnue_rejetee() {
        assert!(compile("[pct_integration.cousin]", &perim()).is_err());
    }

    #[test]
    fn evaluate_elim_ic_min_ratio() {
        // INTEG_PA = 0.6, INTEG_EN = 1.0 → MIN(1, 0.6) = 0.6
        let mut s = HashMap::new();
        s.insert("pct_integration.partner".to_string(), 0.6);
        s.insert("pct_integration.entity".to_string(), 1.0);
        let v = evaluate(
            "MIN(1; SAFE_DIV([pct_integration.partner]; [pct_integration.entity]))",
            &s,
        )
        .unwrap();
        assert!((v - 0.6).abs() < 1e-9);
    }

    #[test]
    fn evaluate_safe_div_sur_zero() {
        // EN absent (=0) → SAFE_DIV = 0 → MIN(1, 0) = 0 (pas de division par zéro).
        let s = HashMap::new();
        let v = evaluate(
            "MIN(1; SAFE_DIV([pct_integration.partner]; [pct_integration.entity]))",
            &s,
        )
        .unwrap();
        assert_eq!(v, 0.0);
    }

    #[test]
    fn evaluate_division_par_zero_non_protegee_erreur() {
        let s = HashMap::new();
        // 1 / [absent=0] → infini → erreur de preview.
        assert!(evaluate("1 / [pct_integration.entity]", &s).is_err());
    }

    #[test]
    fn evaluate_minoritaire() {
        // 1 - [Intérêt entité]
        let mut s = HashMap::new();
        s.insert("pct_interet.entity".to_string(), 0.8);
        let v = evaluate("1 - [pct_interet.entity]", &s).unwrap();
        assert!((v - 0.2).abs() < 1e-9);
    }

    #[test]
    fn operands_listes() {
        let ops = operands("[pct_integration.entity] - [pct_interet.entity]").unwrap();
        assert_eq!(ops.len(), 2);
        assert!(ops.contains(&"pct_integration.entity".to_string()));
    }
}
