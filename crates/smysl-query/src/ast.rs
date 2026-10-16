//! The `sq/1` syntax tree (RFC SMYSL-2.9 §4).
//!
//! One rule shapes every type here: **the tree is what the printer prints and the parser
//! parses, and nothing else.** A node keeps whatever distinction the surface makes — an
//! identifier against a quoted string, an explicit `ascending` against an absent one — because
//! QG-7 asks that `parse(print(parse(q))) == parse(q)`, and a tree that forgets which spelling
//! it came from cannot satisfy it. What the tree does *not* keep is anything the canonical form
//! discards: comments, whitespace and redundant parentheses are gone by the time a node exists.
//!
//! Nothing here knows what a stage means. Meaning is SMYSL-2.5 §3.3's table and the planner
//! TX-P6 step 2 builds.

use smysl_core::diag::Span;

/// A whole query: `using` clauses, an expression, then ordering, shape and output.
///
/// The three tails sit outside [`Expr`] because SMYSL-2.5 §3.2 runs set operations (phase 8)
/// before ordering and shape (phase 9), so a sort inside one arm of a `union` would be
/// discarded. The grammar does not let it be written and this type cannot represent it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    pub header: Vec<Using>,
    pub expr: Expr,
    pub ordering: Vec<Ordering>,
    pub shaping: Vec<Shaping>,
    pub output: Option<OutputStage>,
    pub span: Span,
}

/// A `using` clause: the language version, or a schema pinned to a revision.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Using {
    /// `using sq/1`. A version this build does not support is `SMY-E427`.
    Sq { version: u32, span: Span },
    /// `using x.text/v1@1`.
    Schema {
        id: String,
        revision: u32,
        span: Span,
    },
}

/// Set operations over pipelines, left-associative.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Expr {
    Pipeline(Pipeline),
    Set {
        op: SetOp,
        left: Box<Expr>,
        right: Box<Expr>,
        span: Span,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SetOp {
    Union,
    Intersect,
    Except,
}

/// One pipeline: the five stage classes in the order SMYSL-2.9 §4.2 fixes.
///
/// The order is a property of the *type*, not of a validation pass. A `where` after an `expand`
/// has nowhere to go in this struct, which is why the parser can refuse it with a message
/// naming both stages instead of silently reordering, and why the planner's phase order
/// (SMYSL-2.5 §3.2) is the written order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Pipeline {
    pub scope: Option<Scope>,
    pub source: Option<SourceStage>,
    pub filters: Vec<Filter>,
    pub times: Vec<TimeStage>,
    pub traversals: Vec<Traversal>,
    pub closures: Vec<Closure>,
}

impl Pipeline {
    /// A pipeline must carry at least one stage; an empty one is `SMY-E460`.
    pub fn is_empty(&self) -> bool {
        self.scope.is_none()
            && self.source.is_none()
            && self.filters.is_empty()
            && self.times.is_empty()
            && self.traversals.is_empty()
            && self.closures.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub items: Vec<ScopeItem>,
    pub span: Span,
}

/// Where the question is asked of.
///
/// The plural/singular split is SMYSL-2.5 §3.3's own: a bare plural is every one of that kind
/// in scope, a singular with a colon is that one (SMYSL-2.9 OQ-76).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScopeItem {
    Store(Name),
    Collection(Name),
    Import(Name),
    Channel(Name),
    Channels,
    Library,
    Texts,
}

/// A bareword or a quoted string, kept apart because the printer must give back what it read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Name {
    Ident(String),
    Str(String),
}

/// A source stage and where it was written.
///
/// The span is on the wrapper rather than on each [`Source`] variant because a diagnostic
/// about a source is about the *stage*: `SMY-E461` says this build does not reach phase 5,
/// and the caret belongs under `find "x" ~sem`, not under the string. One span on the stage
/// is also one place for it to be right.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceStage {
    pub source: Source,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Source {
    Find {
        text: Atom,
        engine: Option<Engine>,
        lang: Option<LangSel>,
    },
    AboutEntity(Atom),
    AboutClass(String),
    Text {
        alias: String,
        at: TextAt,
    },
    Uid(Atom),
    View(String),
    Pointer(String),
    Metric(String),
    Question(Atom),
    /// `match <pattern>`: captured with balanced delimiters and **not interpreted**
    /// (SMYSL-2.9 §4.9, OQ-82). The planner refuses it with `SMY-E461`; the bytes are kept so
    /// the printer gives them back unchanged and QG-7 holds.
    Match {
        raw: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Engine {
    Lex,
    Sem,
    Hybrid,
}

/// `lang:any` or `lang:<code>` after a `find` string — a source option, not a field
/// (SMYSL-2.5 §3.1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LangSel {
    Any,
    One(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TextAt {
    /// `text(alias#loc)`. The `#` is part of the locator and never a comment.
    Locator(String),
    /// `text(alias @ range)`.
    Range(String),
}

/// A `where` stage, with the lens modifier SMYSL-2.9 §4.7 attaches to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    pub pred: Pred,
    pub lens_none: bool,
    pub span: Span,
}

/// A predicate.
///
/// `And` and `Or` are n-ary and cannot nest in each other without parentheses, because the
/// grammar has no production for a mixed chain (SMYSL-2.9 §4.5). A parenthesised predicate is
/// its inner node: the printer puts the parentheses back wherever the tree needs them to
/// re-read, which is what keeps QG-7 true without a node for a bracket.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Pred {
    And(Vec<Pred>),
    Or(Vec<Pred>),
    Not(Box<Pred>),
    Cmp {
        field: FieldRef,
        op: CmpOp,
        value: Value,
    },
    Matches(Atom),
    Near {
        text: Atom,
        threshold: Decimal,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    In,
}

/// A field, optionally pinned to a schema revision: `x.text/v1@1:text:mode` (SMYSL-2.5 §3.9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldRef {
    pub pin: Option<SchemaPin>,
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaPin {
    pub id: String,
    pub revision: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Value {
    Str(String),
    Int(i64),
    Dec(Decimal),
    Uid(String),
    Word(String),
    Param(String),
    List(Vec<Value>),
}

/// A string, or a `$parameter` standing in for one.
///
/// Three source positions and two predicates take either, so the distinction has one type
/// rather than an `Option` beside every field.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Atom {
    Str(String),
    Param(String),
}

/// An integer, or a `$parameter` standing in for one.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Count {
    Int(i64),
    Param(String),
}

/// A decimal with at most four fractional places, held as the integer `value × 10_000`.
///
/// There is no binary floating point in `sq/1` (SMYSL-2.9 QG-6). Shortest-round-trip formatting
/// of an `f64` has differed between the four runtimes this grammar is implemented in, which
/// would break QG-7 in exactly one of them; an integer cannot differ. Printing is canonical —
/// trailing zeros are dropped and one fractional digit is always kept — so `0.8000` and `0.8`
/// are one value and one spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Decimal {
    pub scaled: i64,
}

impl Decimal {
    pub const SCALE: i64 = 10_000;

    pub const fn from_scaled(scaled: i64) -> Decimal {
        Decimal { scaled }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeStage {
    pub axis: TimeAxis,
    pub op: TimeOp,
    pub value: TimeValue,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TimeAxis {
    Said,
    Observed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TimeOp {
    During,
    Overlaps,
    Before,
    After,
}

/// An EDTF value, or a range of two. The text is kept verbatim and validated by SMYSL-2.4's
/// parser (`SMY-E410`), which is where EDTF is defined; this crate does not re-implement it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TimeValue {
    At(String),
    Range(String, String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Traversal {
    pub edge: EdgeSpec,
    pub depth: Option<Count>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EdgeSpec {
    /// `expand ->kind`
    Out(String),
    /// `expand <-kind`
    In(String),
    /// `expand -[kind]-`
    Both(String),
    Named(NamedEdge),
}

/// The spelled traversals. `rests on` is two words and the snake_case form is **not** accepted:
/// `rests_on` exists because it is a Rust function name, and SMYSL-2.9 OQ-80 keeps function
/// names out of a language meant to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NamedEdge {
    Grounds,
    Deps,
    RestsOn,
    Dependents,
    Lineage,
    Mentions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Closure {
    pub items: Vec<ClosureItem>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClosureItem {
    Rebuttals,
    Grounds,
    Holders,
    Speakers,
    Spans,
    Datings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Ordering {
    OrderBy {
        keys: Vec<SortKey>,
        lens_none: bool,
        span: Span,
    },
    ReadingOrder {
        span: Span,
    },
    /// `in time order` reads SMYSL-2.4's effective time; `as recorded` selects the observed
    /// axis instead (SMYSL-2.9 OQ-78).
    TimeOrder {
        as_recorded: bool,
        span: Span,
    },
    GroupBy {
        field: FieldRef,
        lens_none: bool,
        span: Span,
    },
    CompareBy {
        field: FieldRef,
        lens_none: bool,
        span: Span,
    },
    AlignByClass {
        policy: Option<String>,
        span: Span,
    },
}

impl Ordering {
    /// Where the stage was written. Every variant carries one; this saves the planner a match
    /// it would otherwise repeat for each diagnostic.
    pub fn span(&self) -> Span {
        match self {
            Ordering::OrderBy { span, .. }
            | Ordering::ReadingOrder { span }
            | Ordering::TimeOrder { span, .. }
            | Ordering::GroupBy { span, .. }
            | Ordering::CompareBy { span, .. }
            | Ordering::AlignByClass { span, .. } => *span,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortKey {
    pub target: SortTarget,
    /// `None` is the default, which is ascending for every key uniformly (SMYSL-2.9 OQ-81).
    /// It is kept apart from `Some(Ascending)` so the printer gives back what was written.
    pub dir: Option<Dir>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SortTarget {
    Salience,
    Field(FieldRef),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Dir {
    Ascending,
    Descending,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Shaping {
    Limit {
        count: Count,
        span: Span,
    },
    Pack {
        tokens: Count,
        as_view: bool,
        span: Span,
    },
}

impl Shaping {
    /// Where the stage was written. See [`Ordering::span`].
    pub fn span(&self) -> Span {
        match self {
            Shaping::Limit { span, .. } | Shaping::Pack { span, .. } => *span,
        }
    }
}

/// An output stage and where it was written. See [`SourceStage`] for why the span is here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputStage {
    pub output: Output,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Output {
    Count,
    Units,
    Json,
    Show,
    Spans,
    Measure(Vec<String>),
}
