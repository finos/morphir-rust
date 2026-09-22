//! Exact positive integer versions for signed TUF metadata.
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{cmp::Ordering, fmt, num::NonZeroU64, str::FromStr};

/// A positive integer version, without a machine-integer upper bound.
///
/// JSON uses an integer token, never a string or floating-point conversion.
/// ```
/// use tough::schema::Version;
/// let version: Version = "18446744073709551615".parse().unwrap();
/// assert_eq!(version.successor().to_string(), "18446744073709551616");
/// ```
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Version(String);

/// The input is not a canonical positive decimal integer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidVersion;
impl fmt::Display for InvalidVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("expected a positive decimal integer version")
    }
}
impl std::error::Error for InvalidVersion {}

impl Version {
    /// Constructs a version from a nonzero machine integer.
    pub fn new(value: u64) -> Option<Self> {
        NonZeroU64::new(value).map(Self::from)
    }

    /// Returns exactly the next positive integer.
    #[must_use]
    pub fn successor(&self) -> Self {
        let mut digits = self.0.as_bytes().to_vec();
        for digit in digits.iter_mut().rev() {
            if *digit < b'9' {
                *digit += 1;
                return Self(digits.into_iter().map(char::from).collect());
            }
            *digit = b'0';
        }
        digits.insert(0, b'1');
        Self(digits.into_iter().map(char::from).collect())
    }

    /// Returns the preceding positive integer, or `None` for one.
    pub fn predecessor(&self) -> Option<Self> {
        if self.0 == "1" {
            return None;
        }
        let mut digits = self.0.as_bytes().to_vec();
        for digit in digits.iter_mut().rev() {
            if *digit > b'0' {
                *digit -= 1;
                break;
            }
            *digit = b'9';
        }
        if digits[0] == b'0' {
            digits.remove(0);
        }
        Some(Self(digits.into_iter().map(char::from).collect()))
    }
}
impl From<NonZeroU64> for Version {
    fn from(value: NonZeroU64) -> Self {
        Self(value.to_string())
    }
}
impl FromStr for Version {
    type Err = InvalidVersion;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.starts_with('0') || value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(InvalidVersion);
        }
        Ok(Self(value.to_owned()))
    }
}
impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0
            .len()
            .cmp(&other.0.len())
            .then_with(|| self.0.cmp(&other.0))
    }
}
impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Serialize for Version {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0
            .parse::<serde_json::Number>()
            .expect("validated decimal integer")
            .serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Box::<serde_json::value::RawValue>::deserialize(deserializer)?
            .get()
            .parse()
            .map_err(serde::de::Error::custom)
    }
}
