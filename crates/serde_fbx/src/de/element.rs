//! Parsed FBX document tree.
//!
//! Both encodings decode into [`Element`]s that borrow names, strings and raw
//! bytes from the input; only numeric arrays are owned, because the binary
//! form must be byte-swapped/inflated and the ASCII form parsed anyway.

use std::borrow::Cow;

use super::{ascii, binary};
use crate::Error;

/// A property value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value<'a> {
    Bool(bool),
    I64(i64),
    F64(f64),
    /// `S` (binary) or a quoted string (ASCII).
    String(Cow<'a, str>),
    Raw(&'a [u8]),
    I64Array(Vec<i64>),
    F64Array(Vec<f64>),
}

impl Value<'_> {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::F64(v) => Some(*v),
            Self::I64(v) => Some(*v as f64),
            Self::Bool(v) => Some(f64::from(u8::from(*v))),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::I64(v) => Some(*v),
            Self::F64(v) => Some(*v as i64),
            Self::Bool(v) => Some(i64::from(*v)),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(v) => Some(v),
            _ => None,
        }
    }

    /// The array as `f64`; borrowed unless stored as integers (ASCII `0,1,2`).
    pub fn as_f64_array(&self) -> Option<Cow<'_, [f64]>> {
        match self {
            Self::F64Array(v) => Some(Cow::Borrowed(v)),
            Self::I64Array(v) => Some(Cow::Owned(v.iter().map(|&v| v as f64).collect())),
            _ => None,
        }
    }

    /// The array as `i64`; borrowed unless stored as floats.
    pub fn as_i64_array(&self) -> Option<Cow<'_, [i64]>> {
        match self {
            Self::I64Array(v) => Some(Cow::Borrowed(v)),
            Self::F64Array(v) => Some(Cow::Owned(v.iter().map(|&v| v as i64).collect())),
            _ => None,
        }
    }
}

/// One FBX node.
#[derive(Debug, Clone, Default)]
pub struct Element<'a> {
    pub name: Cow<'a, str>,
    pub props: Vec<Value<'a>>,
    pub children: Vec<Self>,
}

impl<'a> Element<'a> {
    /// First child with `name`.
    pub fn child(&self, name: &str) -> Option<&Self> {
        self.children.iter().find(|child| child.name == name)
    }

    /// All children with `name`.
    pub fn children_named<'s>(&'s self, name: &'s str) -> impl Iterator<Item = &'s Self> {
        self.children.iter().filter(move |child| child.name == name)
    }

    pub fn prop(&self, index: usize) -> Option<&Value<'a>> {
        self.props.get(index)
    }
}

/// Parses an FBX file into its top-level nodes.
///
/// # Errors
/// If the data is neither a valid binary nor ASCII FBX file.
pub fn parse(bytes: &[u8]) -> Result<Vec<Element<'_>>, Error> {
    if bytes.starts_with(binary::MAGIC) {
        binary::parse(bytes)
    } else {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| load_error("not a binary FBX and not UTF-8 text"))?;
        ascii::parse(text)
    }
}

pub(super) fn load_error(message: impl Into<String>) -> Error {
    Error::LoadFbx {
        message: message.into(),
    }
}
