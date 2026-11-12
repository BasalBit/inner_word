//! Where a binding applies: its key context as text, and what the caller
//! says about the places of the window.
//!
//! GPUI decides whether a binding applies by evaluating a predicate
//! (`"Editor"`, `"LibraryPanel && mode == tree && !TextField"`,
//! `"FindBar > TextField"`) against the key contexts of the elements from
//! the window down to the one that has the keyboard. This crate cannot do
//! that: it has no elements. It works from the **text** of the predicate
//! and from a small description of the window, [`Places`], and answers two
//! questions with them: is text typed where this binding applies, and do
//! two bindings apply in the same place.
//!
//! # A context as text
//!
//! [`Context`] holds the predicate in one canonical spelling, so that the
//! text a crate wrote (`"LibraryPanel && !TextField"`) and the text GPUI
//! prints for the parsed predicate (`"(LibraryPanel && !TextField)"`) are
//! the same context. The canonical spelling is one GPUI parses back to the
//! same predicate. A text this crate cannot read as a predicate is kept as
//! it is and treated as a place of its own.
//!
//! # What is assumed about places
//!
//! Read from a predicate: the names it asks for where the keyboard is
//! (`Editor`, `TextField`; a flag such as `assistant` counts as a name),
//! the names it asks for further out (`FindBar` in `FindBar > TextField`),
//! the names it rules out (`!TextField`, anywhere up to the window), and
//! its conditions (`mode == tree`). From those:
//!
//! - Two contexts with the same canonical text are the **same place**. So
//!   are two bindings without a context.
//! - Two contexts are **apart** when one asks for a name the other rules
//!   out, when they ask for different values of the same key, or when they
//!   share no name and [`Places`] does not say that one lies inside the
//!   other. That last case is an assumption: two differently named places
//!   nobody declared as nested do not have the keyboard at the same time.
//! - A context is **inside** another when it shares a name with it and
//!   asks for everything the other asks for and more
//!   (`LibraryPanel && mode == tree` inside `LibraryPanel`), or when the
//!   other's name is one of its outer names (`FindBar > TextField` inside
//!   `FindBar`).
//! - Two contexts **overlap** when they share a name and each asks for
//!   something the other does not, or when [`Places`] says that one can
//!   lie inside the other (a `TextField` can be in the find bar, and in
//!   other places too).
//! - A binding without a context applies everywhere; every context is
//!   inside it.
//!
//! Who gets a key where two bindings both apply (GPUI's rule, as far as
//! this description can follow it): the one that lies deeper; a binding
//! without a context ranks like the deepest; among bindings at the same
//! depth the one later in the keymap. One refinement is needed for a
//! binding without a context against a named one: a context that never
//! has the keyboard itself, because the keyboard is always in something
//! inside it (a find bar: the keyboard is in its field), loses to the
//! binding without a context whatever the order. [`Places::container`]
//! names such contexts.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The key context a binding applies in, as text, or none (the binding
/// applies wherever the keyboard is). See the [module docs](self).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Context(Option<String>);

impl Context {
    /// No context: the binding applies anywhere in the window.
    pub fn anywhere() -> Self {
        Self(None)
    }

    /// The context with the predicate `text`, in canonical spelling. An
    /// empty text is no context.
    pub fn parse(text: &str) -> Self {
        let text = text.trim();
        if text.is_empty() {
            return Self(None);
        }
        match Predicate::parse(text) {
            Some(predicate) => Self(Some(predicate.to_string())),
            // Not a predicate this crate reads: kept, with its white space
            // made regular.
            None => Self(Some(text.split_whitespace().collect::<Vec<_>>().join(" "))),
        }
    }

    /// [`Context::parse`] for what a GPUI binding gives: `None` for a
    /// binding without a context predicate, else the predicate's text
    /// (`binding.predicate().map(|p| p.to_string())`).
    pub fn from_option(text: Option<&str>) -> Self {
        text.map(Self::parse).unwrap_or_default()
    }

    /// The canonical text of the predicate, which GPUI's
    /// `KeyBindingContextPredicate::parse` reads; `None` without a context.
    pub fn text(&self) -> Option<&str> {
        self.0.as_deref()
    }

    /// Whether the binding applies anywhere (it has no context).
    pub fn is_anywhere(&self) -> bool {
        self.0.is_none()
    }

    /// The names the predicate asks for where the keyboard is, sorted:
    /// `["Editor", "assistant"]` for `Editor && assistant`, `["TextField"]`
    /// for `FindBar > TextField`. Empty without a context.
    pub fn names(&self) -> Vec<String> {
        self.shape()
            .map(|shape| shape.names.into_iter().collect())
            .unwrap_or_default()
    }

    /// What the text of the predicate says, `None` without a context.
    fn shape(&self) -> Option<Shape> {
        let text = self.0.as_deref()?;
        Some(match Predicate::parse(text) {
            Some(predicate) => {
                let mut shape = Shape::default();
                predicate.describe(&mut shape);
                shape
            }
            None => Shape {
                names: BTreeSet::from([text.to_string()]),
                ..Shape::default()
            },
        })
    }
}

impl fmt::Display for Context {
    /// The canonical text, or nothing for no context.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_deref().unwrap_or(""))
    }
}

impl Serialize for Context {
    /// The text, or `null` for no context.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Context {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = Option::<String>::deserialize(deserializer)?;
        Ok(Self::from_option(text.as_deref()))
    }
}

/// A key context predicate, read with GPUI's grammar (gpui 0.2.2,
/// `KeyBindingContextPredicate::parse`): names, `==`, `!=`, `!`, `&&`,
/// `||`, `>` and parentheses, with GPUI's precedences.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Predicate {
    Name(String),
    Equal(String, String),
    NotEqual(String, String),
    Not(Box<Predicate>),
    And(Vec<Predicate>),
    Or(Vec<Predicate>),
    Child(Box<Predicate>, Box<Predicate>),
}

const PRECEDENCE_CHILD: u32 = 1;
const PRECEDENCE_OR: u32 = 2;
const PRECEDENCE_AND: u32 = 3;
const PRECEDENCE_EQ: u32 = 4;
const PRECEDENCE_NOT: u32 = 5;

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
}

impl Predicate {
    fn parse(source: &str) -> Option<Self> {
        let (predicate, rest) = Self::parse_expr(source.trim_start(), 0)?;
        rest.is_empty().then_some(predicate)
    }

    fn parse_expr(source: &str, min_precedence: u32) -> Option<(Self, &str)> {
        let (mut predicate, mut source) = Self::parse_primary(source)?;
        'parse: loop {
            for (operator, precedence) in [
                (">", PRECEDENCE_CHILD),
                ("&&", PRECEDENCE_AND),
                ("||", PRECEDENCE_OR),
                ("==", PRECEDENCE_EQ),
                ("!=", PRECEDENCE_EQ),
            ] {
                if source.starts_with(operator) && precedence >= min_precedence {
                    let rest = source[operator.len()..].trim_start();
                    let (right, rest) = Self::parse_expr(rest, precedence + 1)?;
                    predicate = match operator {
                        ">" => Predicate::Child(Box::new(predicate), Box::new(right)),
                        "&&" => Predicate::and(predicate, right),
                        "||" => Predicate::or(predicate, right),
                        "==" => match (predicate, right) {
                            (Predicate::Name(key), Predicate::Name(value)) => {
                                Predicate::Equal(key, value)
                            }
                            _ => return None,
                        },
                        _ => match (predicate, right) {
                            (Predicate::Name(key), Predicate::Name(value)) => {
                                Predicate::NotEqual(key, value)
                            }
                            _ => return None,
                        },
                    };
                    source = rest;
                    continue 'parse;
                }
            }
            break;
        }
        Some((predicate, source))
    }

    fn parse_primary(source: &str) -> Option<(Self, &str)> {
        let next = source.chars().next()?;
        if next == '(' {
            let (predicate, rest) = Self::parse_expr(source[1..].trim_start(), 0)?;
            let rest = rest.strip_prefix(')')?;
            Some((predicate, rest.trim_start()))
        } else if next == '!' {
            let (predicate, rest) = Self::parse_expr(source[1..].trim_start(), PRECEDENCE_NOT)?;
            Some((Predicate::Not(Box::new(predicate)), rest))
        } else if is_name_char(next) {
            let len = source
                .find(|c: char| !is_name_char(c))
                .unwrap_or(source.len());
            let (name, rest) = source.split_at(len);
            Some((Predicate::Name(name.to_string()), rest.trim_start()))
        } else {
            None
        }
    }

    /// `left && right`, with chains kept flat: the order of evaluation
    /// does not matter for "and".
    fn and(left: Self, right: Self) -> Self {
        let mut parts = Vec::new();
        for side in [left, right] {
            match side {
                Predicate::And(inner) => parts.extend(inner),
                other => parts.push(other),
            }
        }
        Predicate::And(parts)
    }

    fn or(left: Self, right: Self) -> Self {
        let mut parts = Vec::new();
        for side in [left, right] {
            match side {
                Predicate::Or(inner) => parts.extend(inner),
                other => parts.push(other),
            }
        }
        Predicate::Or(parts)
    }

    /// Whether the predicate needs parentheses as a part of a larger one.
    fn is_compound(&self) -> bool {
        matches!(
            self,
            Predicate::And(_) | Predicate::Or(_) | Predicate::Child(..)
        )
    }

    fn write_part(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_compound() {
            write!(f, "({self})")
        } else {
            write!(f, "{self}")
        }
    }

    /// Adds what the predicate asks for to `shape`.
    fn describe(&self, shape: &mut Shape) {
        match self {
            Predicate::Name(name) => {
                shape.names.insert(name.clone());
            }
            Predicate::Equal(key, value) => {
                shape.equal.insert(key.clone(), value.clone());
            }
            Predicate::NotEqual(key, value) => {
                shape.not_equal.insert((key.clone(), value.clone()));
            }
            Predicate::Not(inner) => match inner.as_ref() {
                Predicate::Name(name) => {
                    shape.ruled_out.insert(name.clone());
                }
                Predicate::Equal(key, value) => {
                    shape.not_equal.insert((key.clone(), value.clone()));
                }
                _ => shape.loose = true,
            },
            Predicate::And(parts) => {
                for part in parts {
                    part.describe(shape);
                }
            }
            Predicate::Or(parts) => {
                // Either side may hold: only the names are kept, as places
                // the binding may apply in.
                shape.loose = true;
                for part in parts {
                    let mut side = Shape::default();
                    part.describe(&mut side);
                    shape.names.extend(side.names);
                    shape.outer.extend(side.outer);
                }
            }
            Predicate::Child(parent, child) => {
                child.describe(shape);
                let mut outer = Shape::default();
                parent.describe(&mut outer);
                shape.outer.extend(outer.names);
                shape.outer.extend(outer.outer);
            }
        }
    }
}

impl fmt::Display for Predicate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Predicate::Name(name) => f.write_str(name),
            Predicate::Equal(key, value) => write!(f, "{key} == {value}"),
            Predicate::NotEqual(key, value) => write!(f, "{key} != {value}"),
            Predicate::Not(inner) => match inner.as_ref() {
                Predicate::Name(name) => write!(f, "!{name}"),
                // GPUI reads "!a == b" as "(!a) == b", which is no
                // predicate: everything but a name goes in parentheses.
                other => write!(f, "!({other})"),
            },
            Predicate::And(parts) | Predicate::Or(parts) => {
                let operator = if matches!(self, Predicate::And(_)) {
                    " && "
                } else {
                    " || "
                };
                for (ix, part) in parts.iter().enumerate() {
                    if ix > 0 {
                        f.write_str(operator)?;
                    }
                    part.write_part(f)?;
                }
                Ok(())
            }
            Predicate::Child(parent, child) => {
                parent.write_part(f)?;
                f.write_str(" > ")?;
                child.write_part(f)
            }
        }
    }
}

/// What a predicate asks for, as far as its text says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Shape {
    /// Names asked for where the keyboard is.
    names: BTreeSet<String>,
    /// Names asked for further out (the left side of `>`).
    outer: BTreeSet<String>,
    /// Names ruled out.
    ruled_out: BTreeSet<String>,
    /// `key == value`.
    equal: BTreeMap<String, String>,
    /// `key != value`.
    not_equal: BTreeSet<(String, String)>,
    /// The predicate has an "or" or a negated group: its conditions are
    /// not all known.
    loose: bool,
}

impl Shape {
    /// Whether `self` asks for everything `other` asks for.
    fn asks_for_all_of(&self, other: &Shape) -> bool {
        self.names.is_superset(&other.names)
            && self.outer.is_superset(&other.outer)
            && self.ruled_out.is_superset(&other.ruled_out)
            && self.not_equal.is_superset(&other.not_equal)
            && other
                .equal
                .iter()
                .all(|(key, value)| self.equal.get(key) == Some(value))
    }

    /// Whether the two cannot hold together.
    fn contradicts(&self, other: &Shape) -> bool {
        let one_way = |a: &Shape, b: &Shape| {
            a.names.iter().any(|name| b.ruled_out.contains(name))
                || a.outer.iter().any(|name| b.ruled_out.contains(name))
                || a.equal
                    .iter()
                    .any(|(key, value)| b.not_equal.contains(&(key.clone(), value.clone())))
        };
        one_way(self, other)
            || one_way(other, self)
            || self
                .equal
                .iter()
                .any(|(key, value)| other.equal.get(key).is_some_and(|theirs| theirs != value))
    }
}

/// How the places of two bindings lie to each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Extent {
    /// The same place: wherever one applies the other does.
    Same,
    /// Different places: the two never apply together.
    Apart,
    /// Wherever the first applies the second does too, and the second
    /// applies in more places.
    FirstInsideSecond,
    /// Wherever the second applies the first does too.
    SecondInsideFirst,
    /// They apply together in some places and each alone in others.
    Overlapping,
}

/// Which of two bindings on the same key runs where both apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Rank {
    /// The first, wherever it stands in the keymap (it lies deeper).
    FirstWins,
    /// The second, wherever it stands in the keymap.
    SecondWins,
    /// The one that comes later in the keymap.
    LaterWins,
}

/// How two contexts relate: see [`Places::relation`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Relation {
    /// How the two places lie to each other.
    pub extent: Extent,
    /// Who gets a key where both apply. Without meaning when they are
    /// [`Extent::Apart`].
    pub rank: Rank,
}

/// What the caller knows about the places of the window. Built once by the
/// app, from its key contexts:
///
/// ```
/// use writer_keymap::{Context, Places};
///
/// let places = Places::new()
///     .typing("Editor")
///     .typing("TextField")
///     .inside("TextField", "FindBar")
///     .inside("TextField", "LibraryPanel")
///     .container("FindBar");
/// assert!(places.is_typing(&Context::parse("Editor")));
/// assert!(places.is_typing(&Context::anywhere()));
/// assert!(!places.is_typing(&Context::parse("LibraryPanel && !TextField")));
/// assert!(places.is_typing(&Context::parse("LibraryPanel")));
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Places {
    typing: BTreeSet<String>,
    /// Inner name to the names it can lie inside.
    inside: BTreeMap<String, BTreeSet<String>>,
    containers: BTreeSet<String>,
}

impl Places {
    /// A description that knows nothing yet: no place where text is typed
    /// except "anywhere", no nesting.
    pub fn new() -> Self {
        Self::default()
    }

    /// Says that text is typed where the context `name` has the keyboard
    /// (`"Editor"`, `"TextField"`).
    pub fn typing(mut self, name: &str) -> Self {
        self.typing.insert(name.to_string());
        self
    }

    /// Says that the context `inner` can lie inside the context `outer`:
    /// the keyboard can be in `inner` while `outer` is one of the contexts
    /// around it (`inside("TextField", "FindBar")`). Nesting carries on:
    /// what is inside `inner` is inside `outer` too.
    pub fn inside(mut self, inner: &str, outer: &str) -> Self {
        self.inside
            .entry(inner.to_string())
            .or_default()
            .insert(outer.to_string());
        self
    }

    /// Says that the context `name` never has the keyboard itself: the
    /// keyboard is always in something inside it (a find bar, a picker
    /// with its query field). A binding on such a context loses to a
    /// binding without a context on the same key, whatever the order.
    pub fn container(mut self, name: &str) -> Self {
        self.containers.insert(name.to_string());
        self
    }

    /// Every name `name` lies inside, directly or through others.
    fn outer_of(&self, name: &str) -> BTreeSet<String> {
        let mut found = BTreeSet::new();
        let mut todo = vec![name.to_string()];
        while let Some(current) = todo.pop() {
            for outer in self.inside.get(&current).into_iter().flatten() {
                if found.insert(outer.clone()) {
                    todo.push(outer.clone());
                }
            }
        }
        found
    }

    /// Whether `inner` lies inside `outer` (not the same name).
    fn lies_inside(&self, inner: &str, outer: &str) -> bool {
        inner != outer && self.outer_of(inner).contains(outer)
    }

    /// Whether text is typed somewhere a binding with this context
    /// applies. True for no context (the binding applies in the text
    /// too), for a context that asks for a typing context, and for a
    /// context that has a typing context inside it and does not rule it
    /// out (`LibraryPanel` with its rename field is one,
    /// `LibraryPanel && !TextField` is not). Conditions are not looked at:
    /// `Editor && vim_mode == normal` counts as a place where text is
    /// typed.
    pub fn is_typing(&self, context: &Context) -> bool {
        let Some(shape) = context.shape() else {
            return true;
        };
        if shape.names.iter().any(|name| self.typing.contains(name)) {
            return true;
        }
        self.typing.iter().any(|typing| {
            !shape.ruled_out.contains(typing)
                && shape
                    .names
                    .iter()
                    .any(|name| self.lies_inside(typing, name))
        })
    }

    /// How the places of two contexts lie to each other and which of two
    /// bindings on the same key runs where both apply. The rules and what
    /// they assume are in the [module docs](self).
    pub fn relation(&self, first: &Context, second: &Context) -> Relation {
        let relation = |extent, rank| Relation { extent, rank };
        if first == second {
            return relation(Extent::Same, Rank::LaterWins);
        }
        let (a, b) = match (first.shape(), second.shape()) {
            (None, None) => return relation(Extent::Same, Rank::LaterWins),
            // A binding without a context ranks like the deepest context:
            // a named one keeps a key only where it has the keyboard
            // itself, and only when it comes later.
            (None, Some(b)) => {
                let rank = if self.is_held_from_inside(&b) {
                    Rank::FirstWins
                } else {
                    Rank::LaterWins
                };
                return relation(Extent::SecondInsideFirst, rank);
            }
            (Some(a), None) => {
                let rank = if self.is_held_from_inside(&a) {
                    Rank::SecondWins
                } else {
                    Rank::LaterWins
                };
                return relation(Extent::FirstInsideSecond, rank);
            }
            (Some(a), Some(b)) => (a, b),
        };

        if a.contradicts(&b) {
            return relation(Extent::Apart, Rank::LaterWins);
        }
        if !a.names.is_disjoint(&b.names) {
            // The same context with different conditions: the same depth.
            let extent = match (a.asks_for_all_of(&b), b.asks_for_all_of(&a)) {
                (true, true) => Extent::Same,
                (true, false) => Extent::FirstInsideSecond,
                (false, true) => Extent::SecondInsideFirst,
                (false, false) => Extent::Overlapping,
            };
            return relation(extent, Rank::LaterWins);
        }
        // Deeper by its own text: always inside.
        if !a.outer.is_disjoint(&b.names) {
            return relation(Extent::FirstInsideSecond, Rank::FirstWins);
        }
        if !b.outer.is_disjoint(&a.names) {
            return relation(Extent::SecondInsideFirst, Rank::SecondWins);
        }
        // Deeper by what the caller declared: it can lie inside, and it
        // can be somewhere else too.
        if self.can_lie_inside(&a, &b) {
            return relation(Extent::Overlapping, Rank::FirstWins);
        }
        if self.can_lie_inside(&b, &a) {
            return relation(Extent::Overlapping, Rank::SecondWins);
        }
        relation(Extent::Apart, Rank::LaterWins)
    }

    /// Whether the caller declared one of the names of `inner` (or of its
    /// outer names) as lying inside one of the names `outer` asks for.
    fn can_lie_inside(&self, inner: &Shape, outer: &Shape) -> bool {
        outer.names.iter().any(|name| {
            inner
                .names
                .iter()
                .chain(inner.outer.iter())
                .any(|inner_name| self.lies_inside(inner_name, name))
        })
    }

    /// Whether the keyboard is never on this context itself.
    fn is_held_from_inside(&self, shape: &Shape) -> bool {
        shape
            .names
            .iter()
            .any(|name| self.containers.contains(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(text: &str) -> Context {
        Context::parse(text)
    }

    #[test]
    fn the_canonical_text_is_the_same_for_the_source_and_for_gpui_s_print() {
        for (written, printed) in [
            ("Editor", "Editor"),
            ("LibraryPanel && !TextField", "(LibraryPanel && !TextField)"),
            (
                "LibraryPanel && mode == tree && !TextField",
                "((LibraryPanel && mode == tree) && !TextField)",
            ),
            ("FindBar > TextField", "FindBar > TextField"),
            ("Editor && assistant_quick", "(Editor && assistant_quick)"),
            (
                "Editor && vim_mode == normal",
                "(Editor && vim_mode == normal)",
            ),
            ("a || b && c", "(a || (b && c))"),
            ("!(a && b) > c", "!(a && b) > c"),
            ("mode != trash", "mode != trash"),
        ] {
            assert_eq!(context(written), context(printed), "{written}");
            // What is written is read back as the same context.
            let text = context(written).text().unwrap().to_string();
            assert_eq!(context(&text), context(written), "{written}");
        }
        assert_eq!(
            context("LibraryPanel&&mode==tree  &&  !TextField").text(),
            Some("LibraryPanel && mode == tree && !TextField")
        );
        assert_eq!(context("a || b && c").text(), Some("a || (b && c)"));
        assert_eq!(context("(a || b) && c").text(), Some("(a || b) && c"));
        assert_eq!(context("!(a && b)").text(), Some("!(a && b)"));
        assert_eq!(context("A && B > C").text(), Some("(A && B) > C"));
    }

    #[test]
    fn no_context_and_texts_that_are_not_predicates() {
        assert!(context("").is_anywhere());
        assert!(context("   ").is_anywhere());
        assert_eq!(Context::from_option(None), Context::anywhere());
        assert_eq!(Context::from_option(Some("Editor")), context("Editor"));
        // Kept as it is, a place of its own.
        let odd = context("Editor  &&");
        assert_eq!(odd.text(), Some("Editor &&"));
        assert_eq!(odd.names(), vec!["Editor &&".to_string()]);
    }

    #[test]
    fn names() {
        assert_eq!(
            context("Editor && assistant").names(),
            ["Editor", "assistant"]
        );
        assert_eq!(context("FindBar > TextField").names(), ["TextField"]);
        assert!(Context::anywhere().names().is_empty());
    }

    #[test]
    fn as_json() {
        assert_eq!(serde_json::to_string(&Context::anywhere()).unwrap(), "null");
        assert_eq!(
            serde_json::to_string(&context("(Editor && assistant)")).unwrap(),
            "\"Editor && assistant\""
        );
        let back: Context = serde_json::from_str("\"(Editor && assistant)\"").unwrap();
        assert_eq!(back, context("Editor && assistant"));
        let none: Context = serde_json::from_str("null").unwrap();
        assert!(none.is_anywhere());
    }

    fn the_app() -> Places {
        Places::new()
            .typing("Editor")
            .typing("TextField")
            .inside("Editor", "Workspace")
            .inside("LibraryPanel", "Workspace")
            .inside("TextField", "LibraryPanel")
            .inside("TextField", "FindBar")
            .inside("TextField", "Picker")
            .inside("CommentComposer", "CommentsPanel")
            .inside("Editor", "CommentComposer")
            .container("FindBar")
            .container("Picker")
    }

    #[test]
    fn where_text_is_typed() {
        let places = the_app();
        for text in [
            "Editor",
            "TextField",
            "FindBar > TextField",
            "Editor && assistant",
            "Editor && vim_mode == normal",
            "Workspace",
            "LibraryPanel",
            "FindBar",
            "Picker",
            "CommentsPanel",
        ] {
            assert!(places.is_typing(&context(text)), "{text}");
        }
        assert!(places.is_typing(&Context::anywhere()));
        for text in [
            "LibraryPanel && !TextField",
            "LibraryPanel && mode == trash && !TextField",
            "VersionHistory",
            "CommentThreads",
            "AgentsView",
        ] {
            assert!(!places.is_typing(&context(text)), "{text}");
        }
    }

    #[test]
    fn how_places_lie_to_each_other() {
        let places = the_app();
        let relation = |a: &str, b: &str| places.relation(&context(a), &context(b));
        let extent = |a: &str, b: &str| relation(a, b).extent;

        assert_eq!(extent("Editor", "(Editor)"), Extent::Same);
        assert_eq!(extent("Editor", "VersionHistory"), Extent::Apart);
        assert_eq!(
            extent("LibraryPanel && !TextField", "TextField"),
            Extent::Apart
        );
        assert_eq!(
            extent("LibraryPanel && !TextField", "FindBar > TextField"),
            Extent::Apart
        );
        assert_eq!(
            extent(
                "LibraryPanel && mode == tree && !TextField",
                "LibraryPanel && mode == trash && !TextField"
            ),
            Extent::Apart
        );
        assert_eq!(
            extent(
                "Editor && vim_mode == normal",
                "Editor && vim_mode != normal"
            ),
            Extent::Apart
        );

        // The same context with more conditions: inside, at the same depth.
        let tree = relation(
            "LibraryPanel && mode == tree && !TextField",
            "LibraryPanel && !TextField",
        );
        assert_eq!(tree.extent, Extent::FirstInsideSecond);
        assert_eq!(tree.rank, Rank::LaterWins);
        let chat = relation("Editor", "Editor && assistant");
        assert_eq!(chat.extent, Extent::SecondInsideFirst);
        assert_eq!(chat.rank, Rank::LaterWins);
        let field = relation("FindBar > TextField", "TextField");
        assert_eq!(field.extent, Extent::FirstInsideSecond);
        assert_eq!(field.rank, Rank::LaterWins);
        assert_eq!(
            extent("Editor && assistant", "Editor && read_only"),
            Extent::Overlapping
        );

        // Deeper, by the text and by what was declared.
        let deeper = relation("FindBar > TextField", "FindBar");
        assert_eq!(deeper.extent, Extent::FirstInsideSecond);
        assert_eq!(deeper.rank, Rank::FirstWins);
        let declared = relation("Workspace", "Editor");
        assert_eq!(declared.extent, Extent::Overlapping);
        assert_eq!(declared.rank, Rank::SecondWins);
        // Through two steps: the composer's editor inside the comments panel.
        let composer = relation("Editor", "CommentsPanel");
        assert_eq!(composer.extent, Extent::Overlapping);
        assert_eq!(composer.rank, Rank::FirstWins);
    }

    #[test]
    fn a_binding_without_a_context_against_a_named_one() {
        let places = the_app();
        let anywhere = Context::anywhere();
        assert_eq!(
            places.relation(&anywhere, &anywhere),
            Relation {
                extent: Extent::Same,
                rank: Rank::LaterWins
            }
        );
        // The find bar's field keeps a key bound later (cmd-alt-c today).
        let field = places.relation(&anywhere, &context("FindBar > TextField"));
        assert_eq!(field.extent, Extent::SecondInsideFirst);
        assert_eq!(field.rank, Rank::LaterWins);
        // The find bar itself never has the keyboard: it loses.
        let bar = places.relation(&anywhere, &context("FindBar"));
        assert_eq!(bar.rank, Rank::FirstWins);
        let bar = places.relation(&context("FindBar"), &anywhere);
        assert_eq!(bar.extent, Extent::FirstInsideSecond);
        assert_eq!(bar.rank, Rank::SecondWins);
    }
}
