//! Which numbering instance (`w:num`) and which list definition
//! (`w:abstractNum`) every numbered list item uses.
//!
//! # The limits
//!
//! Word stores a paragraph's list as an index into its table of numbering
//! instances, and that index has room for 2,046 entries (MS-DOC,
//! `sprmPIlfo`: 0x0001 to 0x07FE). Past that, Word shows list items as
//! plain paragraphs. A book can hold more separate numbered lists than
//! that, so neither instances nor definitions may grow with the document
//! without bound.
//!
//! # Instances
//!
//! A list that has to start counting again gets a numbering instance that
//! restarts through `w:lvlOverride` / `w:startOverride`. An instance is
//! reused whenever a reader shows the right number anyway: a nested list
//! restarts by itself after an item of an outer level, and a list that
//! goes on after nested items or an indented paragraph simply continues.
//! Only a list that must restart where nothing restarts it takes a new
//! instance, and at most [`MAX_INSTANCES`] are handed out. Lists beyond
//! that are written with their numbers as text ([`Slot::Literal`]), which
//! looks the same.
//!
//! # Definitions
//!
//! The first [`MAX_DEFINITIONS`]` - 1` instances have a definition each;
//! every later instance shares the last definition. The number of
//! definitions therefore never exceeds a small constant. The start
//! override alone restarts a list in Word, LibreOffice and Google Docs,
//! so one shared definition would be enough for them; but Apple's Quick
//! Look (Finder, Mail, iOS) ignores overrides and counts per definition,
//! and it numbers a second list "3, 4" when both share one. With a
//! definition of their own, the lists of any ordinary document are right
//! there too; only a document with more separate lists than definitions
//! shows continued numbers in Quick Look from that point on.
//!
//! # Two ways of counting
//!
//! Readers do not agree on how instances of one definition relate. Word
//! counts per definition: every instance continues the same counters, and
//! an override restarts a level when its instance first uses it.
//! LibreOffice treats every instance as a list of its own. [`Allocator`]
//! tracks both and reuses an instance only when both give the number the
//! document wants; a new instance carries a start value for every level,
//! so both readings agree on it from its first item on.

/// Levels of a list definition.
pub(crate) const LEVELS: usize = 9;

/// The most numbering instances for numbered lists in one export. With the
/// bullet instance that is under half of what Word can address.
pub(crate) const MAX_INSTANCES: usize = 1000;

/// The most definitions of numbered lists in one export.
pub(crate) const MAX_DEFINITIONS: usize = 128;

/// One numbering instance of a decimal definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Instance {
    /// Which decimal definition it belongs to, from 0.
    pub definition: usize,
    /// The number each level starts at.
    pub starts: [u32; LEVELS],
}

/// How one numbered item is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Slot {
    /// Index into [`Allocator::instances`].
    Instance(usize),
    /// No instance left: the number is written as text.
    Literal,
}

/// Counters of one instance, as a reader that keeps instances apart sees
/// them.
#[derive(Clone, Copy, Default)]
struct Own {
    /// The last number shown per level; `None` when the level starts over.
    counters: [Option<u32>; LEVELS],
    /// Whether the instance has been used at the level.
    used: [bool; LEVELS],
}

/// Hands out numbering instances for the numbered items of an export, in
/// document order.
pub(crate) struct Allocator {
    instances: Vec<Instance>,
    own: Vec<Own>,
    /// Counters of every definition, as Word keeps them.
    shared: Vec<[Option<u32>; LEVELS]>,
    /// The instance last used at each level, and the one used last of all.
    last_at: [Option<usize>; LEVELS],
    last: Option<usize>,
    /// Whether the last numbered item at each level had its number written
    /// as text. The rest of that list is written the same way.
    literal_at: [bool; LEVELS],
    limit: usize,
    definitions: usize,
}

impl Allocator {
    pub fn new() -> Allocator {
        Allocator::with_limits(MAX_INSTANCES, MAX_DEFINITIONS)
    }

    /// At most `limit` instances over at most `definitions` definitions.
    pub fn with_limits(limit: usize, definitions: usize) -> Allocator {
        Allocator {
            instances: Vec::new(),
            own: Vec::new(),
            shared: Vec::new(),
            last_at: [None; LEVELS],
            last: None,
            literal_at: [false; LEVELS],
            limit,
            definitions: definitions.max(1),
        }
    }

    /// The instances handed out so far.
    #[cfg(test)]
    pub fn instances(&self) -> &[Instance] {
        &self.instances
    }

    pub fn into_instances(self) -> Vec<Instance> {
        self.instances
    }

    /// The number a reader shows for the next item of instance `i` at
    /// `level`, when both ways of counting agree on it.
    fn predict(&self, i: usize, level: usize) -> Option<u32> {
        let Instance { definition, starts } = &self.instances[i];
        let start = starts[level];
        let own = &self.own[i];
        let separate = own.counters[level].map_or(start, |n| n + 1);
        let together = if !own.used[level] {
            // The override restarts the level at its first use.
            start
        } else {
            match self.shared[*definition][level] {
                Some(n) => n + 1,
                // Restarted by an outer item. Whether a restart goes back
                // to the override or to the definition's start is not
                // something to rely on, so only a start of 1 is safe.
                None if start == 1 => 1,
                None => return None,
            }
        };
        (separate == together).then_some(separate)
    }

    /// The slot for the next numbered item, at `level`, that the document
    /// numbers `ordinal`.
    pub fn item(&mut self, level: usize, ordinal: u32) -> Slot {
        let level = level.min(LEVELS - 1);
        let ordinal = ordinal.max(1);
        // An item that continues a list written as text stays text, and
        // so does a list nested in one.
        let continues_literal =
            (self.literal_at[level] && ordinal > 1) || self.literal_at[..level].iter().any(|l| *l);
        for deeper in level + 1..LEVELS {
            self.literal_at[deeper] = false;
        }
        let candidates = [self.last_at[level], self.last];
        let reuse = candidates
            .into_iter()
            .flatten()
            .find(|i| !continues_literal && self.predict(*i, level) == Some(ordinal));
        let index = match reuse {
            Some(i) => i,
            None if !continues_literal && self.instances.len() < self.limit => {
                let mut starts = [1; LEVELS];
                starts[level] = ordinal;
                let definition = self.instances.len().min(self.definitions - 1);
                if definition == self.shared.len() {
                    self.shared.push([None; LEVELS]);
                }
                self.instances.push(Instance { definition, starts });
                self.own.push(Own::default());
                self.instances.len() - 1
            }
            None => {
                self.literal_at[level] = true;
                return Slot::Literal;
            }
        };
        self.literal_at[level] = false;
        let own = &mut self.own[index];
        let shared = &mut self.shared[self.instances[index].definition];
        own.used[level] = true;
        own.counters[level] = Some(ordinal);
        shared[level] = Some(ordinal);
        for deeper in level + 1..LEVELS {
            own.counters[deeper] = None;
            shared[deeper] = None;
        }
        self.last_at[level] = Some(index);
        self.last = Some(index);
        Slot::Instance(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slots(items: &[(usize, u32)]) -> (Vec<Slot>, usize) {
        let mut a = Allocator::new();
        let slots = items.iter().map(|(l, n)| a.item(*l, *n)).collect();
        (slots, a.instances().len())
    }

    #[test]
    fn a_nested_list_restarts_in_its_parents_instance() {
        // 1. (1. 2.) 2. (1.) 3.
        let (s, n) = slots(&[(0, 1), (1, 1), (1, 2), (0, 2), (1, 1), (0, 3)]);
        assert_eq!(s, vec![Slot::Instance(0); 6]);
        assert_eq!(n, 1);
    }

    #[test]
    fn a_separate_list_takes_a_new_instance() {
        let (s, n) = slots(&[(0, 1), (0, 2), (0, 1), (0, 2)]);
        assert_eq!(
            s,
            vec![
                Slot::Instance(0),
                Slot::Instance(0),
                Slot::Instance(1),
                Slot::Instance(1)
            ]
        );
        assert_eq!(n, 2);
    }

    #[test]
    fn a_nested_restart_without_an_outer_item_takes_a_new_instance() {
        // 1. (1.) [a bullet at the nested level] (1.) 2.
        let (s, n) = slots(&[(0, 1), (1, 1), (1, 1), (0, 2)]);
        assert_eq!(
            s,
            vec![
                Slot::Instance(0),
                Slot::Instance(0),
                Slot::Instance(1),
                Slot::Instance(0)
            ]
        );
        assert_eq!(n, 2);
        // The new instance restarts every level at 1.
        let mut a = Allocator::new();
        a.item(2, 4);
        assert_eq!(a.instances()[0].starts, [1, 1, 4, 1, 1, 1, 1, 1, 1]);
    }

    #[test]
    fn definitions_are_bounded() {
        let mut a = Allocator::with_limits(10, 3);
        for _ in 0..6 {
            a.item(0, 1);
            a.item(0, 2);
        }
        let definitions: Vec<usize> = a.instances().iter().map(|i| i.definition).collect();
        assert_eq!(definitions, vec![0, 1, 2, 2, 2, 2]);
        // With one definition for everything the result is the same.
        let mut one = Allocator::with_limits(10, 1);
        let mut many = Allocator::with_limits(10, 10);
        for (level, ordinal) in [(0, 1), (1, 1), (1, 1), (1, 2), (0, 2), (1, 1), (0, 1)] {
            assert_eq!(one.item(level, ordinal), many.item(level, ordinal));
        }
    }

    #[test]
    fn an_instance_that_started_deeper_is_not_trusted_at_an_outer_level() {
        // (1.) then an outer item numbered 1, then the nested list again.
        let (s, _) = slots(&[(1, 1), (0, 1), (1, 1), (0, 2), (1, 1)]);
        assert_eq!(s, vec![Slot::Instance(0); 5]);
        // An outer list that goes on after a nested list in another
        // instance returns to its own instance.
        let (s, n) = slots(&[(0, 1), (1, 1), (1, 1), (1, 2), (0, 2), (1, 1)]);
        assert_eq!(n, 2);
        assert_eq!(s[4], Slot::Instance(0));
        assert!(matches!(s[5], Slot::Instance(_)));
    }

    #[test]
    fn past_the_limit_numbers_are_text() {
        let mut a = Allocator::with_limits(2, 1);
        assert_eq!(a.item(0, 1), Slot::Instance(0));
        assert_eq!(a.item(0, 1), Slot::Instance(1));
        assert_eq!(a.item(0, 1), Slot::Literal);
        assert_eq!(a.item(0, 2), Slot::Literal);
        // A list nested in a literal list is literal too, although the
        // last instance could count it.
        assert_eq!(a.item(1, 1), Slot::Literal);
        assert_eq!(a.item(1, 2), Slot::Literal);
        assert_eq!(a.item(0, 3), Slot::Literal);
        assert_eq!(a.instances().len(), 2);
        // A list that began before the limit goes on in its instance.
        let mut a = Allocator::with_limits(1, 1);
        assert_eq!(a.item(0, 1), Slot::Instance(0));
        assert_eq!(a.item(1, 1), Slot::Instance(0));
        assert_eq!(a.item(1, 1), Slot::Literal);
        assert_eq!(a.item(0, 2), Slot::Instance(0));
        assert_eq!(a.item(1, 1), Slot::Instance(0));
    }
}
