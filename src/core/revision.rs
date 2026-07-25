//! Monotonic document revision values.

/// A monotonic version of one document's in-memory content.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Revision(u64);

impl Revision {
    /// The initial revision before any content mutation.
    pub const INITIAL: Self = Self(0);

    /// Creates a revision from its stored numeric value.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the stored numeric value.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Returns the next revision, or `None` if the counter is exhausted.
    pub const fn checked_next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revisions_advance_without_wrapping() {
        assert_eq!(Revision::default(), Revision::INITIAL);
        assert_eq!(Revision::new(41).get(), 41);
        assert_eq!(Revision::new(41).checked_next(), Some(Revision::new(42)));
        assert_eq!(Revision::new(u64::MAX).checked_next(), None);
    }
}
