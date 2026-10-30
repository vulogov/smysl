//! The canonical printer (RFC SMYSL-2.9 §3.4).
//!
//! Not a pretty-printer: a **normaliser**. One query has one canonical text, and that is what
//! makes three things work — a saved query's identity is the hash of this output rather than of
//! whatever was typed (SMYSL-2.5 §3.6), `diff` of two queries shows what changed in the
//! question rather than in the typing, and four implementations can be compared at all.
//!
//! So it discards what the canonical form does not carry — comments, incidental whitespace,
//! redundant parentheses — and emits one stage per line with no `|`. The two laws it owes are
//! QG-7's: `parse(print(parse(q))) == parse(q)` and `print(parse(print(parse(q)))) ==
//! print(parse(q))`, both asserted over every accept fixture.
//!
//! Parentheses are printed **where the tree needs them to read back**, not where they were
//! typed. The tree has no node for a bracket, so `(a)` and `a` are one tree and print one way;
//! `a union (b union c)` keeps its brackets because without them it would re-parse
//! left-associatively into a different tree.

use crate::ast::*;

/// The canonical text of a query.
pub fn print(q: &Query) -> String {
    let mut out = Vec::new();
    for u in &q.header {
        out.push(match u {
            Using::Sq { version, .. } => format!("using sq/{version}"),
            Using::Schema { id, revision, .. } => format!("using {id}@{revision}"),
        });
    }
    expr(&q.expr, &mut out);
    for o in &q.ordering {
        out.push(ordering(o));
    }
    for s in &q.shaping {
        out.push(shaping(s));
    }
    if let Some(o) = &q.output {
        out.push(output(&o.output));
    }
    out.join("\n")
}

fn expr(e: &Expr, out: &mut Vec<String>) {
    match e {
        Expr::Pipeline(p) => pipeline(p, out),
        Expr::Set {
            op, left, right, ..
        } => {
            expr(left, out);
            out.push(
                match op {
                    SetOp::Union => "union",
                    SetOp::Intersect => "intersect",
                    SetOp::Except => "except",
                }
                .to_string(),
            );
            // A set expression on the right must keep its brackets: without them the text
            // re-parses left-associatively into a different tree.
            match right.as_ref() {
                Expr::Set { .. } => {
                    out.push("(".to_string());
                    expr(right, out);
                    out.push(")".to_string());
                }
                other => expr(other, out),
            }
        }
    }
}

fn pipeline(p: &Pipeline, out: &mut Vec<String>) {
    if let Some(s) = &p.scope {
        let items: Vec<String> = s.items.iter().map(scope_item).collect();
        out.push(format!("from {}", items.join(", ")));
    }
    if let Some(s) = &p.source {
        out.push(source(&s.source));
    }
    for f in &p.filters {
        out.push(format!("where {}{}", pred(&f.pred), lens(f.lens_none)));
    }
    for t in &p.times {
        out.push(time_stage(t));
    }
    for t in &p.traversals {
        out.push(traversal(t));
    }
    for c in &p.closures {
        let items: Vec<&str> = c.items.iter().map(closure_item).collect();
        out.push(format!("with {}", items.join(", ")));
    }
}

fn lens(on: bool) -> &'static str {
    if on {
        " with lens none"
    } else {
        ""
    }
}

fn name(n: &Name) -> String {
    match n {
        Name::Ident(s) => s.clone(),
        Name::Str(s) => string(s),
    }
}

fn scope_item(i: &ScopeItem) -> String {
    match i {
        ScopeItem::Store(n) => format!("store:{}", name(n)),
        ScopeItem::Collection(n) => format!("collection:{}", name(n)),
        ScopeItem::Import(n) => format!("import:{}", name(n)),
        ScopeItem::Channel(n) => format!("channel:{}", name(n)),
        ScopeItem::Channels => "channels".to_string(),
        ScopeItem::Library => "library".to_string(),
        ScopeItem::Texts => "texts".to_string(),
    }
}

fn atom(a: &Atom) -> String {
    match a {
        Atom::Str(s) => string(s),
        Atom::Param(p) => format!("${p}"),
    }
}

fn count(c: &Count) -> String {
    match c {
        Count::Int(n) => n.to_string(),
        Count::Param(p) => format!("${p}"),
    }
}

fn source(s: &Source) -> String {
    match s {
        Source::Find { text, engine, lang } => {
            let mut out = format!("find {}", atom(text));
            if let Some(e) = engine {
                out.push_str(match e {
                    Engine::Lex => " ~lex",
                    Engine::Sem => " ~sem",
                    Engine::Hybrid => " ~hybrid",
                });
            }
            match lang {
                Some(LangSel::Any) => out.push_str(" lang:any"),
                Some(LangSel::One(l)) => out.push_str(&format!(" lang:{l}")),
                _ => {}
            }
            out
        }
        Source::AboutEntity(a) => format!("about entity({})", atom(a)),
        Source::AboutClass(u) => format!("about class({u})"),
        Source::Text { alias, at } => match at {
            TextAt::Locator(l) => format!("text({alias}#{l})"),
            TextAt::Range(r) => format!("text({alias} @ {r})"),
        },
        Source::Uid(a) => format!(
            "uid {}",
            match a {
                Atom::Str(s) => s.clone(),
                other => atom(other),
            }
        ),
        Source::View(v) => format!("view {v}"),
        Source::Pointer(s) => format!("pointer({})", string(s)),
        Source::Metric(s) => format!("metric({})", string(s)),
        Source::Question(a) => format!("question {}", atom(a)),
        Source::Match { raw, .. } => format!("match {raw}"),
    }
}

/// A predicate as the printer writes it, for the planner's rewrite lines.
pub(crate) fn pred_text(p: &Pred) -> String {
    pred(p)
}

fn pred(p: &Pred) -> String {
    match p {
        Pred::And(items) => items.iter().map(nested).collect::<Vec<_>>().join(" and "),
        Pred::Or(items) => items.iter().map(nested).collect::<Vec<_>>().join(" or "),
        Pred::Not(inner) => format!("not {}", nested(inner)),
        Pred::Cmp { field, op, value } => format!(
            "{} {} {}",
            field_ref(field),
            match op {
                CmpOp::Eq => "=",
                CmpOp::Ne => "!=",
                CmpOp::Lt => "<",
                CmpOp::Le => "<=",
                CmpOp::Gt => ">",
                CmpOp::Ge => ">=",
                CmpOp::In => "in",
            },
            value_of(value)
        ),
        Pred::Matches(a) => format!("matches {}", atom(a)),
        Pred::Near { text, threshold } => {
            format!("near({}, {})", atom(text), decimal(*threshold))
        }
    }
}

/// A chain inside another chain, or inside `not`, had parentheses in the source and needs them
/// back: the grammar refuses a mixed chain without them (§4.5).
fn nested(p: &Pred) -> String {
    match p {
        Pred::And(_) | Pred::Or(_) => format!("({})", pred(p)),
        other => pred(other),
    }
}

fn field_ref(f: &FieldRef) -> String {
    match &f.pin {
        Some(p) => format!("{}@{}:{}", p.id, p.revision, f.name),
        None => f.name.clone(),
    }
}

fn value_of(v: &Value) -> String {
    match v {
        Value::Str(s) => string(s),
        Value::Int(n) => n.to_string(),
        Value::Dec(d) => decimal(*d),
        Value::Uid(u) => u.clone(),
        Value::Word(w) => w.clone(),
        Value::Param(p) => format!("${p}"),
        Value::List(items) => format!(
            "[{}]",
            items.iter().map(value_of).collect::<Vec<_>>().join(", ")
        ),
    }
}

/// Trailing zeros dropped, one fractional digit always kept, so `0.8000` and `0.8` are one
/// value and one spelling (QG-6).
pub(crate) fn decimal(d: Decimal) -> String {
    let neg = d.scaled < 0;
    let n = d.scaled.unsigned_abs();
    let whole = n / Decimal::SCALE as u64;
    let mut frac = format!("{:04}", n % Decimal::SCALE as u64);
    while frac.len() > 1 && frac.ends_with('0') {
        frac.pop();
    }
    format!("{}{whole}.{frac}", if neg { "-" } else { "" })
}

fn string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

fn time_stage(t: &TimeStage) -> String {
    format!(
        "{} {} {}",
        match t.axis {
            TimeAxis::Said => "said",
            TimeAxis::Observed => "observed",
        },
        match t.op {
            TimeOp::During => "during",
            TimeOp::Overlaps => "overlaps",
            TimeOp::Before => "before",
            TimeOp::After => "after",
        },
        match &t.value {
            TimeValue::At(a) => a.clone(),
            TimeValue::Range(a, b) => format!("{a}..{b}"),
        }
    )
}

fn traversal(t: &Traversal) -> String {
    let edge = match &t.edge {
        EdgeSpec::Out(k) => format!("->{k}"),
        EdgeSpec::In(k) => format!("<-{k}"),
        EdgeSpec::Both(k) => format!("-[{k}]-"),
        EdgeSpec::Named(n) => match n {
            NamedEdge::Grounds => "grounds",
            NamedEdge::Deps => "deps",
            NamedEdge::RestsOn => "rests on",
            NamedEdge::Dependents => "dependents",
            NamedEdge::Lineage => "lineage",
            NamedEdge::Mentions => "mentions",
        }
        .to_string(),
    };
    let walk = match &t.depth {
        Some(d) => format!("expand {edge} depth {}", count(d)),
        None => format!("expand {edge}"),
    };
    if t.only {
        format!("{walk} only")
    } else {
        walk
    }
}

fn closure_item(i: &ClosureItem) -> &'static str {
    match i {
        ClosureItem::Rebuttals => "rebuttals",
        ClosureItem::Grounds => "grounds",
        ClosureItem::Holders => "holders",
        ClosureItem::Speakers => "speakers",
        ClosureItem::Spans => "spans",
        ClosureItem::Datings => "datings",
    }
}

fn ordering(o: &Ordering) -> String {
    match o {
        Ordering::OrderBy {
            keys, lens_none, ..
        } => {
            let ks: Vec<String> = keys
                .iter()
                .map(|k| {
                    let t = match &k.target {
                        SortTarget::Salience => "salience".to_string(),
                        SortTarget::Field(f) => field_ref(f),
                    };
                    match k.dir {
                        Some(Dir::Ascending) => format!("{t} ascending"),
                        Some(Dir::Descending) => format!("{t} descending"),
                        _ => t,
                    }
                })
                .collect();
            format!("order by {}{}", ks.join(", "), lens(*lens_none))
        }
        Ordering::ReadingOrder { .. } => "in reading order".to_string(),
        Ordering::TimeOrder { as_recorded, .. } => {
            if *as_recorded {
                "in time order as recorded".to_string()
            } else {
                "in time order".to_string()
            }
        }
        Ordering::GroupBy {
            field, lens_none, ..
        } => format!("group by {}{}", field_ref(field), lens(*lens_none)),
        Ordering::CompareBy {
            field, lens_none, ..
        } => format!("compare by {}{}", field_ref(field), lens(*lens_none)),
        Ordering::AlignByClass { policy, .. } => match policy {
            Some(p) => format!("align by class {p}"),
            None => "align by class".to_string(),
        },
    }
}

fn shaping(s: &Shaping) -> String {
    match s {
        Shaping::Limit { count: c, .. } => format!("limit {}", count(c)),
        Shaping::Pack {
            tokens, as_view, ..
        } => format!(
            "pack {} tokens{}",
            count(tokens),
            if *as_view { " as view" } else { "" }
        ),
    }
}

fn output(o: &Output) -> String {
    match o {
        Output::Count => "count".to_string(),
        Output::Units => "units".to_string(),
        Output::Json => "json".to_string(),
        Output::Show => "show".to_string(),
        Output::Spans => "spans".to_string(),
        Output::Measure(ids) => format!("measure {}", ids.join(", ")),
    }
}
