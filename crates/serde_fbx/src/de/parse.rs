//! FBX document parser (binary and ASCII).
//!
//! Both formats are decoded into the same [`Element`] tree:
//!
//! ```text
//! Name: prop, prop, ... {
//!     Child: ...
//! }
//! ```
//!
//! Binary nodes use 32-bit offsets before version 7500 and 64-bit offsets
//! from 7500. Array properties may be zlib compressed (Blender always does).

use crate::Error;

/// A property value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    I64(i64),
    F64(f64),
    /// `S` (binary) or a quoted string (ASCII).
    String(String),
    Raw(Vec<u8>),
    I64Array(Vec<i64>),
    F64Array(Vec<f64>),
}

impl Value {
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

    pub fn to_f64_vec(&self) -> Option<Vec<f64>> {
        match self {
            Self::F64Array(v) => Some(v.clone()),
            Self::I64Array(v) => Some(v.iter().map(|&v| v as f64).collect()),
            _ => None,
        }
    }

    pub fn to_i64_vec(&self) -> Option<Vec<i64>> {
        match self {
            Self::I64Array(v) => Some(v.clone()),
            Self::F64Array(v) => Some(v.iter().map(|&v| v as i64).collect()),
            _ => None,
        }
    }
}

/// One FBX node.
#[derive(Debug, Clone, Default)]
pub struct Element {
    pub name: String,
    pub props: Vec<Value>,
    pub children: Vec<Self>,
}

impl Element {
    /// First child with `name`.
    pub fn child(&self, name: &str) -> Option<&Self> {
        self.children.iter().find(|child| child.name == name)
    }

    /// All children with `name`.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Self> {
        self.children.iter().filter(move |child| child.name == name)
    }

    pub fn prop(&self, index: usize) -> Option<&Value> {
        self.props.get(index)
    }
}

/// Parses an FBX file into its top-level nodes.
///
/// # Errors
/// If the data is neither a valid binary nor ASCII FBX file.
pub fn parse(bytes: &[u8]) -> Result<Vec<Element>, Error> {
    if bytes.starts_with(BINARY_MAGIC) {
        BinaryReader::new(bytes).parse()
    } else {
        let text = std::str::from_utf8(bytes).map_err(|_| load_error("not a binary FBX and not UTF-8 text"))?;
        AsciiParser::new(text).parse()
    }
}

fn load_error(message: impl Into<String>) -> Error {
    Error::LoadFbx {
        message: message.into(),
    }
}

// ---------------------------------------------------------------------------
// Binary

const BINARY_MAGIC: &[u8] = b"Kaydara FBX Binary  \x00";

struct BinaryReader<'a> {
    data: &'a [u8],
    pos: usize,
    wide: bool,
}

impl<'a> BinaryReader<'a> {
    const fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            wide: false,
        }
    }

    fn parse(mut self) -> Result<Vec<Element>, Error> {
        self.pos = 23;
        let version = self.u32()?;
        self.wide = version >= 7500;

        let mut nodes = Vec::new();
        while let Some(node) = self.node()? {
            nodes.push(node);
        }
        Ok(nodes)
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], Error> {
        let end = self
            .pos
            .checked_add(len)
            .filter(|&end| end <= self.data.len())
            .ok_or_else(|| load_error(format!("unexpected end of file at {:#x}", self.pos)))?;
        let bytes = &self.data[self.pos..end];
        self.pos = end;
        Ok(bytes)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        let bytes = self.take(N)?;
        let mut array = [0; N];
        array.copy_from_slice(bytes);
        Ok(array)
    }

    fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, Error> {
        self.array().map(u32::from_le_bytes)
    }

    fn offset(&mut self) -> Result<u64, Error> {
        if self.wide {
            self.array().map(u64::from_le_bytes)
        } else {
            self.u32().map(u64::from)
        }
    }

    /// Reads one node, or `None` for a null record.
    fn node(&mut self) -> Result<Option<Element>, Error> {
        if self.pos >= self.data.len() {
            return Ok(None);
        }

        let end_offset = self.offset()? as usize;
        let num_props = self.offset()?;
        let _props_len = self.offset()?;
        let name_len = self.u8()? as usize;
        if end_offset == 0 {
            return Ok(None);
        }

        let name = String::from_utf8_lossy(self.take(name_len)?).into_owned();
        let mut props = Vec::with_capacity(num_props.min(64) as usize);
        for _ in 0..num_props {
            props.push(self.value()?);
        }

        let mut children = Vec::new();
        while self.pos < end_offset {
            match self.node()? {
                Some(child) => children.push(child),
                None => break, // null record terminating the children
            }
        }

        if end_offset > self.data.len() {
            return Err(load_error(format!("node {name} exceeds the file")));
        }
        self.pos = end_offset;

        Ok(Some(Element {
            name,
            props,
            children,
        }))
    }

    fn value(&mut self) -> Result<Value, Error> {
        Ok(match self.u8()? {
            b'C' => Value::Bool(self.u8()? != 0),
            b'Y' => Value::I64(i64::from(i16::from_le_bytes(self.array()?))),
            b'I' => Value::I64(i64::from(i32::from_le_bytes(self.array()?))),
            b'L' => Value::I64(i64::from_le_bytes(self.array()?)),
            b'F' => Value::F64(f64::from(f32::from_le_bytes(self.array()?))),
            b'D' => Value::F64(f64::from_le_bytes(self.array()?)),
            b'S' => {
                let len = self.u32()? as usize;
                Value::String(String::from_utf8_lossy(self.take(len)?).into_owned())
            }
            b'R' => {
                let len = self.u32()? as usize;
                Value::Raw(self.take(len)?.to_vec())
            }
            b'b' => Value::I64Array(self.array_values(1, |b| i64::from(b[0]))?),
            b'i' => Value::I64Array(
                self.array_values(4, |b| i64::from(i32::from_le_bytes([b[0], b[1], b[2], b[3]])))?,
            ),
            b'l' => Value::I64Array(self.array_values(8, |b| {
                i64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
            })?),
            b'f' => Value::F64Array(self.array_values(4, |b| {
                f64::from(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            })?),
            b'd' => Value::F64Array(self.array_values(8, |b| {
                f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
            })?),
            code => {
                return Err(load_error(format!(
                    "unknown property type {:?} at {:#x}",
                    code as char,
                    self.pos - 1
                )));
            }
        })
    }

    fn array_values<T>(&mut self, stride: usize, decode: impl Fn(&[u8]) -> T) -> Result<Vec<T>, Error> {
        let count = self.u32()? as usize;
        let encoding = self.u32()?;
        let len = self.u32()? as usize;
        let raw = self.take(len)?;

        let inflated;
        let bytes = match encoding {
            0 => raw,
            1 => {
                inflated = miniz_oxide::inflate::decompress_to_vec_zlib(raw)
                    .map_err(|error| load_error(format!("failed to inflate array: {error:?}")))?;
                &inflated
            }
            _ => return Err(load_error(format!("unknown array encoding {encoding}"))),
        };

        if bytes.len() < count * stride {
            return Err(load_error("array data is shorter than its element count"));
        }
        Ok(bytes.chunks_exact(stride).take(count).map(decode).collect())
    }
}

// ---------------------------------------------------------------------------
// ASCII

#[derive(Debug, Clone, PartialEq)]
enum Token<'a> {
    /// `Name:`
    Key(&'a str),
    /// Bare word, e.g. `T`, `F`, `a` (inside arrays it is a key).
    Word(&'a str),
    Number(&'a str),
    String(String),
    /// `*123`
    Count,
    Comma,
    Open,
    Close,
}

struct AsciiParser<'a> {
    tokens: Vec<Token<'a>>,
    pos: usize,
}

impl<'a> AsciiParser<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            tokens: tokenize(text),
            pos: 0,
        }
    }

    fn parse(mut self) -> Result<Vec<Element>, Error> {
        let mut nodes = Vec::new();
        while self.pos < self.tokens.len() {
            nodes.push(self.node()?);
        }
        Ok(nodes)
    }

    fn peek(&self) -> Option<&Token<'a>> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<Token<'a>> {
        let token = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        token
    }

    fn node(&mut self) -> Result<Element, Error> {
        let name = match self.next() {
            Some(Token::Key(name)) => name.to_owned(),
            other => return Err(load_error(format!("expected a node name, found {other:?}"))),
        };

        let mut props = Vec::new();
        loop {
            match self.peek() {
                Some(Token::Count) => {
                    // `Name: *N { a: v, v, ... }`
                    self.pos += 1;
                    self.next(); // N
                    props.push(self.array_body()?);
                    return Ok(Element {
                        name,
                        props,
                        children: Vec::new(),
                    });
                }
                Some(Token::Number(_) | Token::String(_) | Token::Word(_)) => {
                    let token = self.next();
                    props.push(match token {
                        Some(Token::Number(number)) => parse_number(number),
                        Some(Token::String(string)) => Value::String(string),
                        Some(Token::Word("T" | "Y")) => Value::Bool(true),
                        Some(Token::Word("F" | "N")) => Value::Bool(false),
                        Some(Token::Word(word)) => Value::String(word.to_owned()),
                        _ => unreachable!(),
                    });
                    if self.peek() == Some(&Token::Comma) {
                        self.pos += 1;
                    }
                }
                _ => break,
            }
        }

        let mut children = Vec::new();
        if self.peek() == Some(&Token::Open) {
            self.pos += 1;
            while !matches!(self.peek(), Some(Token::Close) | None) {
                children.push(self.node()?);
            }
            self.pos += 1;
        }

        Ok(Element {
            name,
            props,
            children,
        })
    }

    /// `{ a: v, v, ... }`
    fn array_body(&mut self) -> Result<Value, Error> {
        if self.next() != Some(Token::Open) {
            return Err(load_error("expected '{' after array count"));
        }

        let mut numbers = Vec::new();
        let mut is_float = false;
        while let Some(token) = self.next() {
            match token {
                Token::Close => break,
                Token::Number(number) => {
                    is_float |= number.contains(['.', 'e', 'E']);
                    numbers.push(number);
                }
                Token::Key(_) | Token::Comma => {}
                other => return Err(load_error(format!("unexpected {other:?} in array"))),
            }
        }

        Ok(if is_float {
            Value::F64Array(numbers.iter().map(|n| n.parse().unwrap_or(0.0)).collect())
        } else {
            Value::I64Array(numbers.iter().map(|n| n.parse().unwrap_or(0)).collect())
        })
    }
}

fn parse_number(number: &str) -> Value {
    if number.contains(['.', 'e', 'E']) {
        Value::F64(number.parse().unwrap_or(0.0))
    } else {
        number
            .parse()
            .map_or_else(|_| Value::F64(number.parse().unwrap_or(0.0)), Value::I64)
    }
}

fn tokenize(text: &str) -> Vec<Token<'_>> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b';' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'{' => {
                tokens.push(Token::Open);
                i += 1;
            }
            b'}' => {
                tokens.push(Token::Close);
                i += 1;
            }
            b',' => {
                tokens.push(Token::Comma);
                i += 1;
            }
            b'*' => {
                tokens.push(Token::Count);
                i += 1;
            }
            b'"' => {
                let start = i + 1;
                i = start;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += 1;
                }
                let raw = &text[start..i.min(bytes.len())];
                tokens.push(Token::String(raw.replace("&quot;", "\"")));
                i += 1;
            }
            _ if c.is_ascii_whitespace() => i += 1,
            _ => {
                let start = i;
                while i < bytes.len() && !matches!(bytes[i], b',' | b'{' | b'}' | b':' | b'"' | b';')
                    && !bytes[i].is_ascii_whitespace()
                {
                    i += 1;
                }
                let word = &text[start..i];
                if i < bytes.len() && bytes[i] == b':' {
                    tokens.push(Token::Key(word));
                    i += 1;
                } else if word.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '+' || c == '.') {
                    tokens.push(Token::Number(word));
                } else if word.is_empty() {
                    i += 1; // stray ':'
                } else {
                    tokens.push(Token::Word(word));
                }
            }
        }
    }

    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_nodes_and_arrays() {
        let text = "; comment\nObjects:  {\n\tModel: 1, \"Model::Pelvis\", \"LimbNode\" {\n\t\tShading: T\n\t\tKeyTime: *2 {\n\t\t\ta: 0,10\n\t\t}\n\t}\n}\n";
        let nodes = parse(text.as_bytes()).unwrap();
        let model = nodes[0].child("Model").unwrap();
        assert_eq!(model.props[0], Value::I64(1));
        assert_eq!(model.props[1].as_str(), Some("Model::Pelvis"));
        assert_eq!(model.child("Shading").unwrap().props[0], Value::Bool(true));
        assert_eq!(model.child("KeyTime").unwrap().props[0], Value::I64Array(vec![0, 10]));
    }
}
