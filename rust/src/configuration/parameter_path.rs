//! Canonical identities for arbitrarily nested JSON parameters.

use std::fmt;

/// One non-empty JSON object path represented as unescaped key segments.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) struct ParameterPath {
    segments: Box<[Box<str>]>,
    identifier: Box<str>,
}

impl ParameterPath {
    pub(crate) fn root(key: impl Into<Box<str>>) -> Self {
        Self::from_segments(vec![key.into()]).expect("root parameter key is nonempty")
    }

    pub(crate) fn from_segments(segments: Vec<Box<str>>) -> Option<Self> {
        if segments.is_empty() {
            return None;
        }
        let identifier = encode_identifier(&segments).into_boxed_str();
        Some(Self {
            segments: segments.into_boxed_slice(),
            identifier,
        })
    }

    /// Parses a canonical non-root JSON Pointer.
    pub(super) fn parse(key: &str) -> Option<Self> {
        let pointer = key.strip_prefix('/')?;
        let segments = pointer
            .split('/')
            .map(decode_pointer_segment)
            .collect::<Option<Vec<_>>>()?;
        Self::from_segments(segments.into_iter().map(String::into_boxed_str).collect())
    }

    pub(crate) fn appended(&self, segment: impl Into<Box<str>>) -> Self {
        let mut segments = self.segments.to_vec();
        segments.push(segment.into());
        Self::from_segments(segments).expect("appending a nonempty path segment remains valid")
    }

    pub(super) fn segments(&self) -> impl ExactSizeIterator<Item = &str> {
        self.segments.iter().map(AsRef::as_ref)
    }

    pub(super) fn is_ancestor_of(&self, other: &Self) -> bool {
        self.segments.len() < other.segments.len()
            && self
                .segments
                .iter()
                .zip(other.segments.iter())
                .all(|(left, right)| left == right)
    }

    /// Stable external JSON Pointer identifier.
    pub(super) fn identifier(&self) -> &str {
        &self.identifier
    }
}

impl fmt::Display for ParameterPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.identifier())
    }
}

fn encode_identifier(segments: &[Box<str>]) -> String {
    let mut output = String::new();
    for segment in segments {
        output.push('/');
        for character in segment.chars() {
            match character {
                '~' => output.push_str("~0"),
                '/' => output.push_str("~1"),
                _ => output.push(character),
            }
        }
    }
    output
}

fn decode_pointer_segment(segment: &str) -> Option<String> {
    let mut output = String::with_capacity(segment.len());
    let mut characters = segment.chars();
    while let Some(character) = characters.next() {
        if character != '~' {
            output.push(character);
            continue;
        }
        match characters.next()? {
            '0' => output.push('~'),
            '1' => output.push('/'),
            _ => return None,
        }
    }
    Some(output)
}
