//! The standard extension schemas (spec §5, SMYSL-2.3 A-11).
//!
//! **Registered, not kernel.** A-11 registers `x.text/v1` so that implementations agree on its
//! names, and then says plainly that being registered buys it nothing else: every rule of spec
//! §5 (rule X) applies to it exactly as to a schema somebody invents this afternoon. Holder,
//! mode and speaker stay here and do not become kernel fields (D-6, OQ-6) — the kernel does not
//! know what a speaker is, and a format that grew a field for every research question would stop
//! being a kernel.
//!
//! **Why the names needed one home.** They were already in use: `x.text/same-as` as a constant
//! in `smysl-graph`, `x.text/quotes` as another in `smysl-text`, `x.text/translates` spelled out
//! in a test. Three spellings of one vocabulary, none of which could be checked against the
//! declaration, because there was no declaration. A-11's whole purpose is that two
//! implementations agree on these strings, and the way to keep that is to have one place they
//! are written and have every reader of them point at it.
//!
//! # What this module does not ship yet
//!
//! Appendix C's declaration carries a `fields:` map, and the structure that puts a field map on
//! the wire — schema declaration key 4 gaining a shape, key 5 gaining lenses — is **A-7**, which
//! G0 did not accept: it stays for discussion with A-10's remainder, A-12 rule N and A-14. So
//! [`text::FIELDS`] is the table in code, available to the converter that writes `text:holder`
//! and `text:mode` from structure (SMYSL-2.4 §3.4), and [`text::decl`] leaves `payload_shape`
//! absent. That is the pre-A-7 reading of the format and not a gap: key 4 has been opaque CBOR
//! that no implementation reads since the field existed, and writing a structure into it before
//! the structure is agreed would put bytes in a store that a later reading would have to
//! either honour or break.
//!
//! `x.query/v1` is the other schema A-11 registers. It was *not* here until TX-P8 step 1 for
//! the same kind of reason: G0 accepted A-11's `x.text/v1` and left `x.query/v1` open, and it
//! arrived with the saved query (SMYSL-2.5 §3.6), which is the first thing that writes one.

use crate::ids::SchemaId;
use crate::types::{RelKind, SchemaDecl};

/// The type of a payload key, in A-7's vocabulary.
///
/// Spelled out even though no field of `x.text/v1` is a `list<T>`, because the vocabulary is
/// closed and a partial copy of a closed set is the kind of thing that is quietly wrong later.
/// There is deliberately no encoder: putting one here would be implementing A-7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum FieldType {
    Text,
    Int,
    Bool,
    /// A canonical uid: 32 bytes in CBOR, `b3:` text at the surface (A-7).
    Uid,
    /// An EDTF value, levels 0 and 1, stored as the string the source states.
    Edtf,
    /// One of a named set. The set is [`Field::values`].
    Enum,
    /// A homogeneous list. `&'static` rather than boxed so the table stays a `const`.
    List(&'static FieldType),
}

impl FieldType {
    /// The spelling A-7 gives this type.
    pub fn as_str(&self) -> &'static str {
        match self {
            FieldType::Text => "text",
            FieldType::Int => "int",
            FieldType::Bool => "bool",
            FieldType::Uid => "uid",
            FieldType::Edtf => "edtf",
            FieldType::Enum => "enum",
            // The one case that cannot be a static string for every inner type. Only the
            // types A-7 admits appear, so the arms are exhaustive over what can be written.
            FieldType::List(inner) => match inner {
                FieldType::Text => "list<text>",
                FieldType::Int => "list<int>",
                FieldType::Bool => "list<bool>",
                FieldType::Uid => "list<uid>",
                FieldType::Edtf => "list<edtf>",
                FieldType::Enum => "list<enum>",
                FieldType::List(_) => "list<list>",
            },
        }
    }
}

/// One payload key of a declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Field {
    /// The payload key, as it is spelled in a payload map.
    pub key: &'static str,
    pub ty: FieldType,
    /// The admitted set, for [`FieldType::Enum`]. Empty otherwise.
    pub values: &'static [&'static str],
    /// Values admitted *besides* the type (A-7 key 6). `text:holder` is a `uid` that also
    /// admits the literal `text`, which is how a unit says "the text itself holds this" rather
    /// than naming a unit that does.
    pub literals: &'static [&'static str],
}

impl Field {
    /// Whether a value is one this field admits by its `values` or its `literals`.
    ///
    /// Answers nothing about a `uid` or an `edtf` — those are grammars, and the crates that own
    /// them are the ones that can say. This is the part of validation a table can do.
    pub fn admits_literally(&self, value: &str) -> bool {
        self.literals.contains(&value)
            || (self.ty == FieldType::Enum && self.values.contains(&value))
    }
}

/// `x.text/v1` — the schema for text research (A-11, Appendix C).
pub mod text {
    use super::{Field, FieldType, RelKind, SchemaDecl, SchemaId};
    use crate::error::ShapeError;
    use crate::ids::LangTag;
    use crate::types::{SourceKind, SourceRef, Status, UnitCore, UnitCoreBuilder};

    /// The schema's own id.
    pub const ID: &str = "x.text/v1";

    /// The revision this build declares. A-12 rule N governs what a later one may change.
    pub const VERSION: u32 = 1;

    /// An occurrence in time: what happened, where a part says it did.
    pub const EVENT: &str = "x.text/event";
    /// A person, place, group, work, object or concept the text names.
    ///
    /// The one type with a normative rule of its own: A-2.4 rule 7 forbids it `span`,
    /// `manifest`, `published` and `grounds`, and fixes its source at
    /// `alias:<expression alias>`, so that one entity named in fifty extraction windows is one
    /// unit rather than fifty.
    pub const ENTITY: &str = "x.text/entity";
    /// A recurring subject, as against a single occurrence.
    pub const THEME: &str = "x.text/theme";
    /// A text's reference to another text.
    pub const CITATION: &str = "x.text/citation";
    /// A statement about the corpus rather than about the world.
    pub const META: &str = "x.text/meta";

    /// The five types, in Appendix C's order.
    pub const TYPES: &[&str] = &[EVENT, ENTITY, THEME, CITATION, META];

    /// Two units are the same proposition. The edge TX-P7's engines propose and classes read.
    pub const SAME_AS: &str = "x.text/same-as";
    pub const ENTAILS: &str = "x.text/entails";
    pub const CONTRADICTS: &str = "x.text/contradicts";
    pub const TRANSLATES: &str = "x.text/translates";
    /// One passage quotes another. Read as a *said*-axis ordering by rule E.
    pub const QUOTES: &str = "x.text/quotes";
    pub const ALLUDES: &str = "x.text/alludes";
    pub const MENTIONS: &str = "x.text/mentions";
    pub const REPLIES: &str = "x.text/replies";
    pub const PRECEDES: &str = "x.text/precedes";
    pub const SUMMARISES: &str = "x.text/summarises";

    /// The ten relation kinds, in Appendix C's order.
    pub const RELATIONS: &[&str] = &[
        SAME_AS,
        ENTAILS,
        CONTRADICTS,
        TRANSLATES,
        QUOTES,
        ALLUDES,
        MENTIONS,
        REPLIES,
        PRECEDES,
        SUMMARISES,
    ];

    /// The values of `text:mode`.
    ///
    /// `asserted` is not the default and must not be treated as one: S0 measured reported
    /// speech extracted *as* asserted in 11.1–24.5% of attribution-bearing units, which is why
    /// SMYSL-2.4 §3.4 has the converter write this field from the structure a reader marked
    /// rather than from a model's self-report.
    pub const MODES: &[&str] = &[
        "asserted",
        "reported",
        "hypothetical",
        "negated",
        "interrogative",
        "imperative",
        "quoted",
    ];

    /// The values of `text:kind`.
    ///
    /// **Not inside an entity's identity.** A-2.4 rule 7 makes an entity's identity a function
    /// of its name, its language and its expression; a `kind` in the payload would be inside the
    /// uid and would split one entity into two the moment two windows disagreed about whether
    /// Jerusalem is a place or a concept.
    pub const KINDS: &[&str] = &["person", "place", "group", "work", "object", "concept"];

    /// The payload keys, in Appendix C's order.
    ///
    /// In code and not on the wire: the structure that would carry this in schema declaration
    /// key 4 is A-7, which G0 did not accept. See the module header.
    pub const FIELDS: &[Field] = &[
        Field {
            key: "text:holder",
            ty: FieldType::Uid,
            values: &[],
            literals: &["text"],
        },
        Field {
            key: "text:mode",
            ty: FieldType::Enum,
            values: MODES,
            literals: &[],
        },
        Field {
            key: "text:speaker",
            ty: FieldType::Uid,
            values: &[],
            literals: &[],
        },
        Field {
            key: "text:locator",
            ty: FieldType::Text,
            values: &[],
            literals: &[],
        },
        Field {
            key: "text:when",
            ty: FieldType::Edtf,
            values: &[],
            literals: &[],
        },
        Field {
            key: "text:kind",
            ty: FieldType::Enum,
            values: KINDS,
            literals: &[],
        },
        Field {
            key: "meta:field",
            ty: FieldType::Text,
            values: &[],
            literals: &[],
        },
        Field {
            key: "meta:value",
            ty: FieldType::Text,
            values: &[],
            literals: &[],
        },
    ];

    /// The prefix of an entity unit's `source.reference` (A-2.4 rule 7).
    ///
    /// An entity's source names the **expression**, by alias, and not a part: the whole point of
    /// the rule is that the unit is not an offset into any one piece of text. The alias is the
    /// library's own name for the expression, so `alias:kjv/1769` is a reference a store can
    /// resolve and a reader can read, which a bare name would not be.
    pub const ALIAS_REF: &str = "alias:";

    /// The reference an entity of this expression carries.
    pub fn alias_reference(alias: &str) -> String {
        format!("{ALIAS_REF}{alias}")
    }

    /// The expression alias a reference names, if it is an alias reference.
    ///
    /// No clash with the four library identities: those are `t3:`, `m3:`, `d3:` and `r3:`, and
    /// `alias` is not a 26–52 character base32 string.
    pub fn alias_of(reference: &str) -> Option<&str> {
        reference.strip_prefix(ALIAS_REF)
    }

    /// The field with this key, if the schema declares one.
    pub fn field(key: &str) -> Option<&'static Field> {
        FIELDS.iter().find(|f| f.key == key)
    }

    /// The declaration, as a record a store can hold.
    ///
    /// `payload_shape` is absent, for the reason in the module header: the shape is A-7's and
    /// A-7 is not accepted. Everything else is Appendix C.
    ///
    /// The ids are constructed rather than parsed, so this cannot fail and needs no `expect` in
    /// a path a store writes. That the constants are nevertheless well-formed under the grammar
    /// is a test, not an assumption — `SchemaId::parse` and `RelKind::parse` are run over every
    /// one of them in this module's tests.
    pub fn decl() -> SchemaDecl {
        let mut d = SchemaDecl::new(SchemaId::Extension(ID.to_string()), VERSION);
        d.types = TYPES
            .iter()
            .map(|t| SchemaId::Extension((*t).to_string()))
            .collect();
        d.relations = RELATIONS
            .iter()
            .map(|r| RelKind::Extension((*r).to_string()))
            .collect();
        d
    }

    /// Whether this schema id is one of the five types this schema declares.
    pub fn declares(id: &SchemaId) -> bool {
        TYPES.contains(&id.as_str())
    }

    // -----------------------------------------------------------------------
    // Rule 7 - entities
    // -----------------------------------------------------------------------

    /// Why an entity unit could not be built.
    ///
    /// Two reasons and no third: the alias is not an alias, or the name is not a gist. A local
    /// enum rather than a `ShapeError` arm because the first reason is not a shape defect — the
    /// unit would be well-formed, it would just name an expression no library could have.
    #[derive(Debug, Clone, PartialEq, Eq)]
    #[non_exhaustive]
    pub enum EntityError {
        /// The alias does not satisfy the alias grammar (`smysl_core::is_alias`).
        ///
        /// Refused rather than normalised. The reference is inside the uid, so a producer that
        /// lowercased `KJV/1769` here would give the same entity two identities depending on how
        /// the caller happened to type the expression's name.
        Alias(String),
        /// The name could not be a gist: empty, or longer than the granularity admits.
        Shape(ShapeError),
    }

    impl core::fmt::Display for EntityError {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            match self {
                EntityError::Alias(a) => write!(f, "`{a}` is not an expression alias"),
                EntityError::Shape(e) => write!(f, "{e}"),
            }
        }
    }

    impl std::error::Error for EntityError {}

    /// An entity unit, under A-2.4 rule 7.
    ///
    /// The three arguments are the whole of the identity, which is the rule stated as a signature:
    /// the name becomes the gist, the language becomes core key 9, the alias becomes the reference,
    /// and there is no fourth parameter through which a caller could add anything else. Two windows
    /// of the same expression that name the same entity in the same language therefore produce the
    /// same uid, and `smysl merge` collapses them without being asked to.
    ///
    /// **Status is `cited` and not the caller's.** It cannot be `measured` — nothing was measured —
    /// and it cannot be `inferred` or `derived`, because both require `grounds` and rule 7 forbids
    /// them, so `UnitCoreBuilder` would refuse. That leaves `cited`, which is also the true one: the
    /// expression names this entity, and the source says which expression. Were it the caller's, it
    /// would be inside the uid and the same entity from two windows could differ by it.
    ///
    /// **No payload.** `text:kind` exists in the declaration and is deliberately not written: the
    /// payload is inside the uid, so a `kind` would split Jerusalem into a place and a concept the
    /// first time two windows disagreed. A kind belongs on an assertion *about* the entity, which is
    /// a unit with grounds and no rule against them.
    pub fn entity(
        name: &str,
        alias: &str,
        lang: Option<&LangTag>,
    ) -> Result<UnitCore, EntityError> {
        if !crate::ids::is_alias(alias) {
            return Err(EntityError::Alias(alias.to_string()));
        }
        let source = SourceRef::new(SourceKind::Doc, alias_reference(alias));
        let b = UnitCoreBuilder::new(SchemaId::Extension(ENTITY.to_string()), name, Status::Cited)
            .source(source);
        let b = match lang {
            Some(tag) => b.lang(tag.clone()),
            None => b,
        };
        b.build().map_err(EntityError::Shape)
    }

    /// Whether a unit breaks rule 7: an entity carrying what an entity may not carry.
    ///
    /// The answer is a list of field names rather than a bool, because the diagnostic has to say
    /// *which* — a producer told only that its entity is wrong has four fields to guess between.
    /// Empty for a unit that is not an entity at all: rule 7 says nothing about those.
    pub fn rule_7_violations(unit: &UnitCore) -> Vec<&'static str> {
        if unit.schema.as_str() != ENTITY {
            return Vec::new();
        }
        let mut out = Vec::new();
        if let Some(source) = unit.source.as_ref() {
            if source.span().is_some() {
                out.push("span");
            }
            if source.manifest().is_some() {
                out.push("manifest");
            }
            if source.published.is_some() {
                out.push("published");
            }
            if alias_of(&source.reference).is_none() {
                out.push("source.reference");
            }
        } else {
            // Rule 7 fixes the source, so an entity without one has not met it either. `cited`
            // without a source is `SMY-E032` and unconstructible through the builder, but a unit
            // decoded from a store is not constructed through the builder.
            out.push("source.reference");
        }
        if !unit.grounds.is_empty() {
            out.push("grounds");
        }
        out
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn every_name_is_well_formed_under_the_grammar() {
            assert!(SchemaId::parse(ID).is_ok(), "{ID}");
            for t in TYPES {
                assert!(SchemaId::parse(t).is_ok(), "{t}");
            }
            for r in RELATIONS {
                assert!(RelKind::parse(r).is_ok(), "{r}");
            }
        }

        /// The ids `decl` builds are the ids the grammar would have parsed. Constructing the
        /// variant directly is what keeps `decl` infallible, and this is the price of that.
        #[test]
        fn the_declaration_agrees_with_the_parser() {
            let d = decl();
            assert_eq!(d.id, SchemaId::parse(ID).expect("a well-formed id"));
            let parsed: Vec<SchemaId> = TYPES
                .iter()
                .map(|t| SchemaId::parse(t).expect("a well-formed id"))
                .collect();
            assert_eq!(d.types, parsed);
            let kinds: Vec<RelKind> = RELATIONS
                .iter()
                .map(|r| RelKind::parse(r).expect("a well-formed kind"))
                .collect();
            assert_eq!(d.relations, kinds);
        }

        /// A-11: registered is not kernel. A declaration that redefined a kernel type or kernel
        /// relation kind would be `SMY-E012`, and the check that says so is the one run here.
        #[test]
        fn the_declaration_redefines_nothing_in_the_kernel() {
            assert!(!decl().redefines_kernel());
        }

        #[test]
        fn the_shape_waits_for_a_7() {
            assert!(decl().payload_shape.is_none());
        }

        #[test]
        fn the_field_table_is_appendix_c() {
            assert_eq!(FIELDS.len(), 8);
            assert_eq!(
                field("text:holder").map(|f| f.ty),
                Some(FieldType::Uid),
                "a uid that also admits a literal"
            );
            assert!(field("text:holder")
                .expect("declared")
                .admits_literally("text"));
            assert!(field("text:mode")
                .expect("declared")
                .admits_literally("reported"));
            assert!(!field("text:mode")
                .expect("declared")
                .admits_literally("claimed"));
            assert_eq!(field("text:when").map(|f| f.ty), Some(FieldType::Edtf));
            assert!(field("x.text/holder").is_none(), "a type is not a field");
        }

        #[test]
        fn an_entity_is_its_name_its_language_and_its_expression() {
            let en = LangTag::new("en").expect("a tag");
            let u = entity("Jerusalem", "kjv/1769", Some(&en)).expect("an entity");
            assert_eq!(u.schema.as_str(), ENTITY);
            assert_eq!(u.gist, "Jerusalem");
            assert_eq!(u.status, Status::Cited);
            assert_eq!(u.lang.as_ref(), Some(&en));
            let source = u.source.as_ref().expect("rule 7 fixes the source");
            assert_eq!(source.reference, "alias:kjv/1769");
            assert_eq!(source.kind, SourceKind::Doc);
            // The four fields rule 7 forbids.
            assert!(source.span().is_none());
            assert!(source.manifest().is_none());
            assert!(source.published.is_none());
            assert!(u.grounds.is_empty());
            // And nothing else is in the identity.
            assert!(u.body.is_none());
            assert!(u.detail.is_none());
            assert!(u.payload.is_none());
            assert!(u.deps.is_empty());
            assert!(rule_7_violations(&u).is_empty());
        }

        /// The exit clause of TX-P5 step 2, in the smallest form it has: the same entity named
        /// from fifty windows is one unit. The windows differ in everything an extraction window
        /// can differ in, and none of it reaches the function, which is the point of the
        /// signature.
        #[test]
        fn the_same_entity_from_fifty_windows_is_one_uid() {
            let ru = LangTag::new("ru").expect("a tag");
            let uids: std::collections::BTreeSet<crate::Uid> = (0..50)
                .map(|_| {
                    let u = entity("Иерусалим", "sinodal/1876", Some(&ru)).expect("an entity");
                    crate::canonical_uid(&u)
                })
                .collect();
            assert_eq!(uids.len(), 1, "fifty windows, one entity");
        }

        /// Three things that *are* different entities, so that the collapse above is not simply
        /// a function that ignores its arguments.
        #[test]
        fn name_language_and_expression_each_separate_two_entities() {
            let en = LangTag::new("en").expect("a tag");
            let ru = LangTag::new("ru").expect("a tag");
            let base = entity("Jerusalem", "kjv/1769", Some(&en)).expect("an entity");
            let by_name = entity("Jericho", "kjv/1769", Some(&en)).expect("an entity");
            let by_lang = entity("Jerusalem", "kjv/1769", Some(&ru)).expect("an entity");
            let by_expr = entity("Jerusalem", "sinodal/1876", Some(&en)).expect("an entity");
            let no_lang = entity("Jerusalem", "kjv/1769", None).expect("an entity");
            let all = [&base, &by_name, &by_lang, &by_expr, &no_lang];
            let uids: std::collections::BTreeSet<crate::Uid> =
                all.iter().map(|u| crate::canonical_uid(u)).collect();
            assert_eq!(uids.len(), 5, "and an absent language is a fifth answer");
        }

        #[test]
        fn an_alias_that_is_not_one_is_refused_rather_than_normalised() {
            assert_eq!(
                entity("Jerusalem", "KJV/1769", None),
                Err(EntityError::Alias("KJV/1769".to_string())),
                "the reference is inside the uid, so case cannot be fixed quietly"
            );
            assert!(matches!(
                entity("", "kjv/1769", None),
                Err(EntityError::Shape(ShapeError::MissingGist))
            ));
        }

        #[test]
        fn rule_7_names_the_field_that_broke_it() {
            let en = LangTag::new("en").expect("a tag");
            let good = entity("Jerusalem", "kjv/1769", Some(&en)).expect("an entity");

            // A unit that is not an entity is not rule 7's business, whatever it carries.
            let claim = UnitCoreBuilder::new(crate::KernelType::Claim, "a claim", Status::Cited)
                .source(SourceRef::new(SourceKind::Doc, "t3:nope"))
                .build()
                .expect("a claim");
            assert!(rule_7_violations(&claim).is_empty());

            // An entity whose reference names a part rather than an expression.
            let mut wrong = good.clone();
            wrong.source = Some(SourceRef::new(SourceKind::Doc, "t3:nope"));
            assert_eq!(rule_7_violations(&wrong), vec!["source.reference"]);

            // An entity with a published date it may not have.
            let mut dated = good.clone();
            let mut source = dated.source.clone().expect("a source");
            source.published = Some("1769".to_string());
            dated.source = Some(source);
            assert_eq!(rule_7_violations(&dated), vec!["published"]);

            // An entity with grounds.
            let mut grounded = good.clone();
            grounded.grounds.insert(crate::canonical_uid(&claim));
            assert_eq!(rule_7_violations(&grounded), vec!["grounds"]);

            // And one with no source at all, which a decoder can produce and a builder cannot.
            let mut bare = good;
            bare.source = None;
            assert_eq!(rule_7_violations(&bare), vec!["source.reference"]);
        }

        #[test]
        fn an_alias_reference_round_trips() {
            let r = alias_reference("kjv/1769");
            assert_eq!(r, "alias:kjv/1769");
            assert_eq!(alias_of(&r), Some("kjv/1769"));
            assert_eq!(
                alias_of("t3:aaaa"),
                None,
                "a part reference is not an alias one"
            );
            assert_eq!(
                alias_of("alias:"),
                Some(""),
                "the grammar is the caller's to check"
            );
        }

        #[test]
        fn the_entity_type_is_the_one_with_a_rule() {
            assert!(declares(&SchemaId::Extension(ENTITY.to_string())));
            assert!(!declares(&SchemaId::Extension("x.text/v1".to_string())));
            assert!(!declares(&SchemaId::from(crate::KernelType::Claim)));
        }
    }
}

/// `x.query/v1` — the saved query (A-11, Appendix C; SMYSL-2.5 §3.6).
///
/// One type and one relation kind. The type is the unit a query is saved as; the relation is
/// the edge from it to each unit of its result, and it is an *extension* kind on purpose:
/// SMYSL-2.5 §3.6 and OQ-49 record that kernel `answers` carries salience rank, so linking
/// results with it would raise their salience and change every later `order by salience` —
/// including the rerun of the query that was saved. `EdgeKind::carries_support` treats an
/// extension kind as non-supporting, which is what makes a saved query inert.
pub mod query {
    use super::{Field, FieldType, RelKind, SchemaDecl, SchemaId};

    /// The schema's own id.
    pub const ID: &str = "x.query/v1";

    /// The revision this build declares.
    pub const VERSION: u32 = 1;

    /// A query that has planned and run to completion, with what it found.
    pub const SAVED: &str = "x.query/saved";

    /// The one type.
    pub const TYPES: &[&str] = &[SAVED];

    /// From a saved query to one unit of its result. Inert for salience (OQ-49).
    pub const ANSWERS: &str = "x.query/answers";

    /// The one relation kind.
    pub const RELATIONS: &[&str] = &[ANSWERS];

    /// The eight payload keys, in Appendix C's order; the eighth, `query:per-partition`, is
    /// OQ-84's. In code and not on the wire, as
    /// [`super::text::FIELDS`] is, and for the same reason: A-7.
    pub const FIELDS: &[Field] = &[
        Field {
            key: "query:text",
            ty: FieldType::Text,
            values: &[],
            literals: &[],
        },
        Field {
            key: "query:recipe",
            ty: FieldType::Text,
            values: &[],
            literals: &[],
        },
        Field {
            key: "query:models",
            ty: FieldType::List(&FieldType::Text),
            values: &[],
            literals: &[],
        },
        Field {
            key: "query:result-hash",
            ty: FieldType::Text,
            values: &[],
            literals: &[],
        },
        Field {
            key: "query:sq",
            ty: FieldType::Text,
            values: &[],
            literals: &[],
        },
        Field {
            key: "query:schemas",
            ty: FieldType::List(&FieldType::Text),
            values: &[],
            literals: &[],
        },
        Field {
            key: "query:lens-chain",
            ty: FieldType::Text,
            values: &[],
            literals: &[],
        },
        Field {
            key: "query:per-partition",
            ty: FieldType::Int,
            values: &[],
            literals: &[],
        },
    ];

    /// The field a payload key names, if the schema declares one.
    pub fn field(key: &str) -> Option<&'static Field> {
        FIELDS.iter().find(|f| f.key == key)
    }

    /// The declaration, as a record a store can hold. Infallible for the reason
    /// [`super::text::decl`] is.
    pub fn decl() -> SchemaDecl {
        let mut d = SchemaDecl::new(SchemaId::Extension(ID.to_string()), VERSION);
        d.types = TYPES
            .iter()
            .map(|t| SchemaId::Extension((*t).to_string()))
            .collect();
        d.relations = RELATIONS
            .iter()
            .map(|r| RelKind::Extension((*r).to_string()))
            .collect();
        d
    }

    /// Whether this schema id is the one type this schema declares.
    pub fn declares(id: &SchemaId) -> bool {
        TYPES.contains(&id.as_str())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn every_name_is_well_formed_under_the_grammar() {
            assert!(SchemaId::parse(ID).is_ok(), "{ID}");
            for t in TYPES {
                assert!(SchemaId::parse(t).is_ok(), "{t}");
            }
            for r in RELATIONS {
                assert!(RelKind::parse(r).is_ok(), "{r}");
            }
        }

        #[test]
        fn the_declaration_agrees_with_the_parser() {
            let d = decl();
            assert_eq!(d.id, SchemaId::parse(ID).expect("a well-formed id"));
            assert_eq!(
                d.types,
                vec![SchemaId::parse(SAVED).expect("a well-formed id")]
            );
            assert_eq!(
                d.relations,
                vec![RelKind::parse(ANSWERS).expect("a well-formed kind")]
            );
        }

        /// The result edge is not the kernel's `answers`, and it does not carry support: that
        /// is the whole of OQ-49, and it is asserted on the kind rather than argued.
        #[test]
        fn the_result_edge_is_not_kernel_answers_and_carries_no_support() {
            let k = RelKind::parse(ANSWERS).expect("a well-formed kind");
            assert!(matches!(k, RelKind::Extension(_)));
            assert!(!k.carries_support());
            assert!(!decl().redefines_kernel());
        }

        #[test]
        fn the_shape_waits_for_a_7() {
            assert!(decl().payload_shape.is_none());
        }

        #[test]
        fn the_field_table_is_appendix_c() {
            assert_eq!(FIELDS.len(), 8);
            assert_eq!(field("query:text").map(|f| f.ty), Some(FieldType::Text));
            assert_eq!(
                field("query:per-partition").map(|f| f.ty),
                Some(FieldType::Int)
            );
            assert_eq!(
                field("query:models").map(|f| f.ty),
                Some(FieldType::List(&FieldType::Text))
            );
            assert_eq!(
                field("query:schemas").map(|f| f.ty),
                Some(FieldType::List(&FieldType::Text))
            );
            assert!(field("x.query/saved").is_none(), "a type is not a field");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_type_spells_itself() {
        assert_eq!(FieldType::List(&FieldType::Text).as_str(), "list<text>");
        assert_eq!(FieldType::Uid.as_str(), "uid");
    }
}
