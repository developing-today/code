//! turso_queue::claim_filter — claim filter language (parser + SQL compiler).
//!
//! Split from turso_queue::claim for #16255 (production-line
//! guard): a sequential, behavior-preserving module split. The
//! parent module re-exports every item, so all crate paths are
//! unchanged.

use super::*;

/// The pick path's claim UPDATE (#7068, #7617): keyed to the
/// picked row's id and re-checking the full pick predicate at claim
/// instant, so a claim that loses the race for a row affects 0 rows
/// and the caller re-picks — it can never be handed the winner's
/// row. Extracted as a builder so the regression tests below pin
/// the statement's shape without a live store.
/// One-queue claim matching (#8097 one-queue model, Drew
/// 2026-11-10): there is ONE fleet queue. A claim matches the
/// UNION of two sides' requirements, each side filtering on the
/// other's metadata:
///
///   * the TASK's requirements filter claimers: the lane-ledger
///     platform limitation (PLATFORM_LIMIT_PICK_CLAUSE, #8217) and
///     the payload `requires` object enforced by
///     TASK_REQUIREMENTS_CLAUSE below — `requires.platform` (a
///     PLATFORM_FAMILIES family), `requires.lane` (a lane id),
///     `requires.pool` (a pool name), `requires.model` (a model
///     id, or "provider/model"). A claimer that does not declare
///     the required attribute does not meet the requirement.
///   * the CLAIMER's requirements filter tasks: the optional
///     stream scope (the request's `queue`) and the boolean
///     filter expression (the request's `filter`, spelled
///     --filter on the CLI) compiled by filter_sql below.
///
/// By default BOTH sides are empty: a task carries no
/// requirements and a claim passes no queue and no filter, and
/// the claim is a pure priority-order pick over every available
/// row in the one queue. The claimer's identity (worker, lane,
/// platform family, pool, model) is recorded on every claim —
/// the claim stamps, unchanged — and is used ONLY to satisfy a
/// task's own declared requirements, never as a filter in its
/// own right. The registry's per-lane queue column no longer
/// scopes what a lane may claim: claim_preamble enforces the
/// lane<->stream binding only when the claim itself names a
/// stream AND the lane's registration binds one (a registration
/// of "*" is the recorded unbound state).
///
/// Filter grammar (recursive descent; AND binds tighter than
/// OR, NOT tightest; parentheses override):
///   expr       := or
///   or         := and (OR and)*
///   and        := unary (AND unary)*
///   unary      := NOT unary | primary
///   primary    := '(' expr ')' | comparison
///   comparison := field (('=' | '!=' | '<' | '<=' | '>' | '>=')
///                        value
///                        | [NOT] IN '(' value (',' value)* ')')
/// Fields (task metadata): repo, queue, pool (the task's stream
/// — pool names the task's queue, as in the claimed event's
/// marks), platform (the row's platform mark), label (membership
/// in the payload's `labels` array), kind (payload `kind`),
/// priority (the row's integer priority), lane / model (the
/// task's own declared requires.lane / requires.model). Ordering
/// comparisons are valid only on priority. A comparison against
/// an absent value is false for '=' / IN / ordering comparisons
/// and true for '!=' / NOT IN (the task does not carry that
/// value).
///
/// `repo` matches the task's repo identity: the payload's `repo`
/// (a full "owner/name") when present, else the stream name with
/// the repo-health- prefix and -pilot / -shadow suffixes
/// stripped (repo-health-code-pilot -> code). A filter value
/// matches the identity exactly or as its final path segment, so
/// `repo = code` matches payload repo "developing-today/code"
/// and stream-derived "code" alike, and `repo != code` is the
/// not-code lanes' claim expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterField {
    Repo,
    Queue,
    Pool,
    Platform,
    Label,
    Kind,
    Priority,
    Lane,
    Model,
}

impl FilterField {
    fn parse(name: &str) -> Option<FilterField> {
        match name.to_ascii_lowercase().as_str() {
            "repo" => Some(FilterField::Repo),
            "queue" => Some(FilterField::Queue),
            "pool" => Some(FilterField::Pool),
            "platform" => Some(FilterField::Platform),
            "label" => Some(FilterField::Label),
            "kind" => Some(FilterField::Kind),
            "priority" => Some(FilterField::Priority),
            "lane" => Some(FilterField::Lane),
            "model" => Some(FilterField::Model),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    In,
    NotIn,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FilterExpr {
    Or(Box<FilterExpr>, Box<FilterExpr>),
    And(Box<FilterExpr>, Box<FilterExpr>),
    Not(Box<FilterExpr>),
    Cmp {
        field: FilterField,
        op: CmpOp,
        values: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FilterTok {
    LParen,
    RParen,
    Comma,
    Op(CmpOp),
    Word(String),
    Str(String),
}

pub(crate) fn filter_tokens(src: &str) -> Result<Vec<FilterTok>, String> {
    let chars: Vec<char> = src.chars().collect();
    let word_char =
        |c: char| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '#' | '@' | '*');
    let mut toks = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        match c {
            c if c.is_whitespace() => i += 1,
            '(' => {
                toks.push(FilterTok::LParen);
                i += 1;
            }
            ')' => {
                toks.push(FilterTok::RParen);
                i += 1;
            }
            ',' => {
                toks.push(FilterTok::Comma);
                i += 1;
            }
            '=' => {
                toks.push(FilterTok::Op(CmpOp::Eq));
                i += 1;
            }
            '!' => {
                if chars.get(i + 1) == Some(&'=') {
                    toks.push(FilterTok::Op(CmpOp::Ne));
                    i += 2;
                } else {
                    return Err("filter: '!' is only valid as part of '!='".to_string());
                }
            }
            '<' => {
                if chars.get(i + 1) == Some(&'=') {
                    toks.push(FilterTok::Op(CmpOp::Le));
                    i += 2;
                } else {
                    toks.push(FilterTok::Op(CmpOp::Lt));
                    i += 1;
                }
            }
            '>' => {
                if chars.get(i + 1) == Some(&'=') {
                    toks.push(FilterTok::Op(CmpOp::Ge));
                    i += 2;
                } else {
                    toks.push(FilterTok::Op(CmpOp::Gt));
                    i += 1;
                }
            }
            '\'' | '"' => {
                let q = c;
                let mut s = String::new();
                let mut j = i + 1;
                while j < chars.len() && chars[j] != q {
                    s.push(chars[j]);
                    j += 1;
                }
                if j >= chars.len() {
                    return Err("filter: unterminated quoted value".to_string());
                }
                toks.push(FilterTok::Str(s));
                i = j + 1;
            }
            c if word_char(c) => {
                let mut s = String::new();
                while i < chars.len() && word_char(chars[i]) {
                    s.push(chars[i]);
                    i += 1;
                }
                toks.push(FilterTok::Word(s));
            }
            other => return Err(format!("filter: unexpected character {other:?}")),
        }
    }
    Ok(toks)
}

pub(crate) struct FilterParser<'a> {
    pub(crate) toks: &'a [FilterTok],
    pub(crate) pos: usize,
}

impl<'a> FilterParser<'a> {
    fn peek(&self) -> Option<&'a FilterTok> {
        self.toks.get(self.pos)
    }

    fn next(&mut self) -> Option<&'a FilterTok> {
        let t = self.toks.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn at_keyword(&self, kw: &str) -> bool {
        matches!(self.peek(), Some(FilterTok::Word(w)) if w.eq_ignore_ascii_case(kw))
    }

    fn parse_or(&mut self) -> Result<FilterExpr, String> {
        let mut lhs = self.parse_and()?;
        while self.at_keyword("or") {
            self.next();
            let rhs = self.parse_and()?;
            lhs = FilterExpr::Or(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> Result<FilterExpr, String> {
        let mut lhs = self.parse_unary()?;
        while self.at_keyword("and") {
            self.next();
            let rhs = self.parse_unary()?;
            lhs = FilterExpr::And(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<FilterExpr, String> {
        if self.at_keyword("not") {
            self.next();
            let inner = self.parse_unary()?;
            return Ok(FilterExpr::Not(Box::new(inner)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<FilterExpr, String> {
        if matches!(self.peek(), Some(FilterTok::LParen)) {
            self.next();
            let e = self.parse_or()?;
            if !matches!(self.next(), Some(FilterTok::RParen)) {
                return Err("filter: expected ')'".to_string());
            }
            return Ok(e);
        }
        self.parse_comparison()
    }

    fn parse_value(&mut self) -> Result<String, String> {
        match self.next() {
            Some(FilterTok::Word(w)) => Ok(w.clone()),
            Some(FilterTok::Str(s)) => Ok(s.clone()),
            other => Err(format!("filter: expected a value, found {other:?}")),
        }
    }

    fn parse_comparison(&mut self) -> Result<FilterExpr, String> {
        let field_name = match self.next() {
            Some(FilterTok::Word(w)) => w.clone(),
            other => return Err(format!("filter: expected a field name, found {other:?}")),
        };
        let field = FilterField::parse(&field_name).ok_or_else(|| {
            format!(
                "filter: unknown field {field_name:?} (fields: repo, queue, pool, \
                 platform, label, kind, priority, lane, model)"
            )
        })?;
        let negated_in = self.at_keyword("not")
            && matches!(self.toks.get(self.pos + 1), Some(FilterTok::Word(w)) if w.eq_ignore_ascii_case("in"));
        if negated_in || self.at_keyword("in") {
            if negated_in {
                self.next();
            }
            self.next();
            if !matches!(self.next(), Some(FilterTok::LParen)) {
                return Err("filter: expected '(' after IN".to_string());
            }
            let mut values = vec![self.parse_value()?];
            while matches!(self.peek(), Some(FilterTok::Comma)) {
                self.next();
                values.push(self.parse_value()?);
            }
            if !matches!(self.next(), Some(FilterTok::RParen)) {
                return Err("filter: expected ')' after the IN list".to_string());
            }
            return Ok(FilterExpr::Cmp {
                field,
                op: if negated_in { CmpOp::NotIn } else { CmpOp::In },
                values,
            });
        }
        let op = match self.next() {
            Some(FilterTok::Op(o)) => *o,
            other => {
                return Err(format!(
                    "filter: expected an operator after {field_name}, found {other:?}"
                ));
            }
        };
        let value = self.parse_value()?;
        Ok(FilterExpr::Cmp {
            field,
            op,
            values: vec![value],
        })
    }
}

/// Parse a claim's filter expression (the request's `filter`
/// field, the CLI's --filter). An absent or empty filter is the
/// default claim — no expression at all — and never reaches
/// here; a malformed one is a usage error, never a silent
/// no-op filter.
pub fn parse_filter(src: &str) -> Result<FilterExpr, String> {
    let toks = filter_tokens(src)?;
    if toks.is_empty() {
        return Err("filter: empty expression".to_string());
    }
    let mut p = FilterParser {
        toks: &toks,
        pos: 0,
    };
    let expr = p.parse_or()?;
    if p.pos != toks.len() {
        return Err("filter: unexpected trailing input".to_string());
    }
    validate_filter(&expr)?;
    Ok(expr)
}

pub(crate) fn validate_filter(expr: &FilterExpr) -> Result<(), String> {
    match expr {
        FilterExpr::Or(a, b) | FilterExpr::And(a, b) => {
            validate_filter(a)?;
            validate_filter(b)
        }
        FilterExpr::Not(a) => validate_filter(a),
        FilterExpr::Cmp { field, op, values } => {
            if values.is_empty() {
                return Err("filter: a comparison needs at least one value".to_string());
            }
            if !matches!(op, CmpOp::In | CmpOp::NotIn) && values.len() != 1 {
                return Err("filter: only IN takes multiple values".to_string());
            }
            match field {
                FilterField::Priority => {
                    for v in values {
                        v.parse::<i64>().map_err(|_| {
                            format!("filter: priority value {v:?} is not an integer")
                        })?;
                    }
                    Ok(())
                }
                _ => {
                    if matches!(op, CmpOp::Lt | CmpOp::Le | CmpOp::Gt | CmpOp::Ge) {
                        return Err(
                            "filter: ordering comparisons are only valid on priority".to_string()
                        );
                    }
                    Ok(())
                }
            }
        }
    }
}

// A tracking issue may live in code while its deliverable lives elsewhere.
// Keep tracking identity (and work-identity dedupe) separate from eligibility.
pub(crate) const IMPLEMENTATION_REPO_SQL: &str =
    "COALESCE(NULLIF(json_extract(payload, '$.implementation_repo'), ''), \
    NULLIF(json_extract(payload, '$.repo'), ''), \
    CASE WHEN queue LIKE 'repo-health-%' \
         THEN replace(replace(replace(queue, 'repo-health-', ''), '-pilot', ''), '-shadow', '') \
         ELSE queue END)";

pub(crate) fn repo_eq_sql(v: &str, args: &mut Vec<Arg>) -> String {
    args.push(Arg::Text(v.to_string()));
    args.push(Arg::Text(v.to_string()));
    format!("({IMPLEMENTATION_REPO_SQL} = ? OR {IMPLEMENTATION_REPO_SQL} LIKE '%/' || ?)")
}

pub(crate) fn label_eq_sql(v: &str, args: &mut Vec<Arg>) -> String {
    args.push(Arg::Text(v.to_string()));
    "EXISTS (SELECT 1 FROM json_each(payload, '$.labels') je WHERE je.value = ?)".to_string()
}

pub(crate) fn text_subject_sql(field: FilterField) -> &'static str {
    match field {
        FilterField::Queue | FilterField::Pool => "queue",
        FilterField::Platform => "platform",
        FilterField::Kind => "json_extract(payload, '$.kind')",
        FilterField::Lane => "json_extract(payload, '$.requires.lane')",
        FilterField::Model => "json_extract(payload, '$.requires.model')",
        FilterField::Repo | FilterField::Label | FilterField::Priority => "",
    }
}

pub(crate) fn cmp_sql(
    field: FilterField,
    op: CmpOp,
    values: &[String],
    args: &mut Vec<Arg>,
) -> String {
    match field {
        FilterField::Repo => {
            let parts: Vec<String> = values.iter().map(|v| repo_eq_sql(v, args)).collect();
            match op {
                CmpOp::Eq | CmpOp::In => format!("({})", parts.join(" OR ")),
                CmpOp::Ne | CmpOp::NotIn => format!("(NOT ({}))", parts.join(" OR ")),
                _ => unreachable!("validated at parse: no ordering on repo"),
            }
        }
        FilterField::Label => {
            let parts: Vec<String> = values.iter().map(|v| label_eq_sql(v, args)).collect();
            match op {
                CmpOp::Eq | CmpOp::In => format!("({})", parts.join(" OR ")),
                CmpOp::Ne | CmpOp::NotIn => format!("(NOT ({}))", parts.join(" OR ")),
                _ => unreachable!("validated at parse: no ordering on label"),
            }
        }
        FilterField::Priority => {
            let nums: Vec<i64> = values
                .iter()
                .map(|v| v.parse::<i64>().unwrap_or(0))
                .collect();
            match op {
                CmpOp::Eq => {
                    args.push(Arg::Int(nums[0]));
                    "priority = ?".to_string()
                }
                CmpOp::Ne => {
                    args.push(Arg::Int(nums[0]));
                    "(priority IS NULL OR priority <> ?)".to_string()
                }
                CmpOp::Lt => {
                    args.push(Arg::Int(nums[0]));
                    "priority < ?".to_string()
                }
                CmpOp::Le => {
                    args.push(Arg::Int(nums[0]));
                    "priority <= ?".to_string()
                }
                CmpOp::Gt => {
                    args.push(Arg::Int(nums[0]));
                    "priority > ?".to_string()
                }
                CmpOp::Ge => {
                    args.push(Arg::Int(nums[0]));
                    "priority >= ?".to_string()
                }
                CmpOp::In => {
                    let ph: Vec<&str> = nums.iter().map(|_| "?").collect();
                    for n in &nums {
                        args.push(Arg::Int(*n));
                    }
                    format!("priority IN ({})", ph.join(", "))
                }
                CmpOp::NotIn => {
                    let ph: Vec<&str> = nums.iter().map(|_| "?").collect();
                    for n in &nums {
                        args.push(Arg::Int(*n));
                    }
                    format!("(priority IS NULL OR priority NOT IN ({}))", ph.join(", "))
                }
            }
        }
        field => {
            let subj = text_subject_sql(field);
            match op {
                CmpOp::Eq => {
                    args.push(Arg::Text(values[0].clone()));
                    format!("{subj} = ?")
                }
                CmpOp::Ne => {
                    args.push(Arg::Text(values[0].clone()));
                    format!("({subj} IS NULL OR {subj} <> ?)")
                }
                CmpOp::In => {
                    let ph: Vec<&str> = values.iter().map(|_| "?").collect();
                    for v in values {
                        args.push(Arg::Text(v.clone()));
                    }
                    format!("{subj} IN ({})", ph.join(", "))
                }
                CmpOp::NotIn => {
                    let ph: Vec<&str> = values.iter().map(|_| "?").collect();
                    for v in values {
                        args.push(Arg::Text(v.clone()));
                    }
                    format!("({subj} IS NULL OR {subj} NOT IN ({}))", ph.join(", "))
                }
                _ => unreachable!("validated at parse: no ordering on text fields"),
            }
        }
    }
}

/// Compile a filter expression to a SQL fragment over the tasks
/// table, appending its arguments in placeholder order. The
/// fragment is a single boolean expression; callers parenthesize
/// it into the pick.
pub(crate) fn filter_sql(expr: &FilterExpr, args: &mut Vec<Arg>) -> String {
    match expr {
        FilterExpr::Or(a, b) => format!("({} OR {})", filter_sql(a, args), filter_sql(b, args)),
        FilterExpr::And(a, b) => {
            format!("({} AND {})", filter_sql(a, args), filter_sql(b, args))
        }
        FilterExpr::Not(a) => format!("(NOT {})", filter_sql(a, args)),
        FilterExpr::Cmp { field, op, values } => cmp_sql(*field, *op, values, args),
    }
}
