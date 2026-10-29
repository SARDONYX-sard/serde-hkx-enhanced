//! ASCII FBX reader.
//!
//! ```text
//! Name: 1, "text", T {      ; properties, `T`/`F` booleans
//!     Child: 2.5
//!     Array: *3 { a: 1,2,3 }
//! }
//! ```
//!
//! Tokenized up front: the grammar needs one token of lookahead (`Name:` vs a
//! value, `*N` arrays) and tokens borrow from the text, so this costs little.

use std::borrow::Cow;

use super::element::{Element, Value, load_error};
use crate::Error;

/// Parses an ASCII FBX file into its top-level nodes.
///
/// # Errors
/// If the structure is malformed (unbalanced braces, bad arrays).
pub fn parse(text: &str) -> Result<Vec<Element<'_>>, Error> {
    let mut parser = Parser {
        tokens: tokenize(text),
        pos: 0,
    };
    let mut nodes = Vec::new();
    while parser.pos < parser.tokens.len() {
        nodes.push(parser.node()?);
    }
    Ok(nodes)
}

#[derive(Debug, Clone, PartialEq)]
enum Token<'a> {
    /// `Name:`
    Key(&'a str),
    /// Bare word, e.g. `T`, `F` (and `a` inside arrays, which is a key).
    Word(&'a str),
    Number(&'a str),
    /// Quoted text; owned only when `&quot;` had to be unescaped.
    String(Cow<'a, str>),
    /// `*` of `*123`
    Count,
    Comma,
    Open,
    Close,
}

struct Parser<'a> {
    tokens: Vec<Token<'a>>,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Token<'a>> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<Token<'a>> {
        let token = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        token
    }

    fn node(&mut self) -> Result<Element<'a>, Error> {
        let name = match self.next() {
            Some(Token::Key(name)) => Cow::Borrowed(name),
            other => return Err(load_error(format!("expected a node name, found {other:?}"))),
        };

        let mut props = Vec::new();
        loop {
            match self.peek() {
                Some(Token::Count) => {
                    // `Name: *N { a: v, v, ... }`: arrays are never mixed with
                    // other properties or children.
                    self.pos += 2; // `*` and N
                    props.push(self.array_body()?);
                    return Ok(Element {
                        name,
                        props,
                        children: Vec::new(),
                    });
                }
                Some(Token::Number(_) | Token::String(_) | Token::Word(_)) => {
                    props.push(match self.next() {
                        Some(Token::Number(number)) => parse_number(number),
                        Some(Token::String(string)) => Value::String(string),
                        Some(Token::Word("T" | "Y")) => Value::Bool(true),
                        Some(Token::Word("F" | "N")) => Value::Bool(false),
                        Some(Token::Word(word)) => Value::String(Cow::Borrowed(word)),
                        _ => unreachable!("matched by peek"),
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

    /// `{ a: v, v, ... }`: integers unless any value has a fraction/exponent.
    fn array_body(&mut self) -> Result<Value<'a>, Error> {
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

fn parse_number(number: &str) -> Value<'_> {
    if number.contains(['.', 'e', 'E']) {
        return Value::F64(number.parse().unwrap_or(0.0));
    }
    // Integers too large for i64 still parse as floats.
    number
        .parse()
        .map_or_else(|_| Value::F64(number.parse().unwrap_or(0.0)), Value::I64)
}

fn tokenize(text: &str) -> Vec<Token<'_>> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        let single = match bytes[i] {
            b'{' => Some(Token::Open),
            b'}' => Some(Token::Close),
            b',' => Some(Token::Comma),
            b'*' => Some(Token::Count),
            _ => None,
        };
        if let Some(token) = single {
            tokens.push(token);
            i += 1;
            continue;
        }

        match bytes[i] {
            b';' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'"' => {
                let start = i + 1;
                let end = text[start..].find('"').map_or(text.len(), |n| start + n);
                let raw = &text[start..end];
                tokens.push(Token::String(if raw.contains("&quot;") {
                    Cow::Owned(raw.replace("&quot;", "\""))
                } else {
                    Cow::Borrowed(raw)
                }));
                i = end + 1;
            }
            c if c.is_ascii_whitespace() => i += 1,
            _ => {
                let start = i;
                while i < bytes.len()
                    && !matches!(bytes[i], b',' | b'{' | b'}' | b':' | b'"' | b';')
                    && !bytes[i].is_ascii_whitespace()
                {
                    i += 1;
                }
                let word = &text[start..i];
                if bytes.get(i) == Some(&b':') {
                    tokens.push(Token::Key(word));
                    i += 1;
                } else if word
                    .starts_with(|c: char| c.is_ascii_digit() || matches!(c, '-' | '+' | '.'))
                {
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
    fn nodes_and_arrays() {
        let text = "; comment\nObjects:  {\n\tModel: 1, \"Model::Pelvis\", \"LimbNode\" {\n\t\tShading: T\n\t\tKeyTime: *2 {\n\t\t\ta: 0,10\n\t\t}\n\t}\n}\n";
        let nodes = parse(text).unwrap();
        let model = nodes[0].child("Model").unwrap();
        assert_eq!(model.props[0], Value::I64(1));
        assert_eq!(model.props[1].as_str(), Some("Model::Pelvis"));
        assert!(
            matches!(model.props[1], Value::String(Cow::Borrowed(_))),
            "strings borrow the input"
        );
        assert_eq!(model.child("Shading").unwrap().props[0], Value::Bool(true));
        assert_eq!(
            model.child("KeyTime").unwrap().props[0],
            Value::I64Array(vec![0, 10])
        );
    }
}
