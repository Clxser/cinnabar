//! Lexical ownership of identifiers in function bodies and initializers.

use super::items::{Param, matching};
use crate::token::Tok;
use std::collections::{HashMap, HashSet};
use std::ops::Range;

/// A local declaration and the part of its scope where its value hides outer bindings.
struct Local {
    declaration: usize,
    visible: Range<usize>,
}

/// Parameters and local scopes used to distinguish global references from shadowing names.
pub(super) struct Bindings {
    parameters: HashSet<String>,
    locals: HashMap<String, Vec<Local>>,
}

impl Bindings {
    /// Resolves local declaration extents in a balanced token stream, including loop scopes.
    pub(super) fn new(toks: &[Tok], parameters: &[Param], types: &HashSet<String>) -> Self {
        let mut types = types.clone();
        types.extend(
            toks.windows(2)
                .filter(|pair| pair[0].ident() == Some("struct"))
                .filter_map(|pair| pair[1].ident().map(str::to_string)),
        );
        let mut scopes = Vec::new();
        for (i, token) in toks.iter().enumerate() {
            if token.is("{") {
                if let Some(end) = matching(toks, i, "{", "}") {
                    scopes.push(i..end + 1);
                }
            } else if matches!(token.ident(), Some("for" | "while" | "switch"))
                && let Some(close) = matching(toks, i + 1, "(", ")")
            {
                scopes.push(i..statement_end(toks, close + 1));
            } else if token.ident() == Some("if")
                && let Some(close) = matching(toks, i + 1, "(", ")")
            {
                let end = statement_end(toks, close + 1);
                scopes.push(close + 1..end);
                if toks.get(end).and_then(Tok::ident) == Some("else") {
                    scopes.push(end + 1..statement_end(toks, end + 1));
                }
            } else if token.ident() == Some("do") {
                scopes.push(i + 1..statement_end(toks, i + 1));
            }
        }
        let mut locals: HashMap<String, Vec<Local>> = HashMap::new();
        for (i, token) in toks.iter().enumerate() {
            let Some(ty) = token.ident() else { continue };
            if !is_type(ty, &types) || (i > 0 && toks[i - 1].is(".")) {
                continue;
            }
            let Some(mut name) = after_arrays(toks, i + 1) else {
                continue;
            };
            if toks.get(name).and_then(Tok::ident).is_none()
                || toks.get(name + 1).is_some_and(|t| t.is("("))
            {
                continue;
            }
            let end = scopes
                .iter()
                .filter(|scope| scope.contains(&name))
                .map(|scope| scope.end)
                .min()
                .unwrap_or(toks.len());
            while let Some(identifier) = toks.get(name).and_then(Tok::ident) {
                let Some(mut cursor) = after_arrays(toks, name + 1) else {
                    break;
                };
                if toks.get(cursor).is_some_and(|t| t.is("=")) {
                    cursor += 1;
                    while cursor < toks.len()
                        && !toks[cursor].is(",")
                        && !toks[cursor].is(";")
                        && !toks[cursor].is(")")
                    {
                        if let Some(close) = group_end(toks, cursor) {
                            cursor = close + 1;
                        } else {
                            cursor += 1;
                        }
                    }
                }
                locals
                    .entry(identifier.to_string())
                    .or_default()
                    .push(Local {
                        declaration: name,
                        visible: cursor..end,
                    });
                if toks.get(cursor).is_some_and(|t| t.is(",")) {
                    name = cursor + 1;
                } else {
                    break;
                }
            }
        }
        Self {
            parameters: parameters.iter().map(|p| p.name.clone()).collect(),
            locals,
        }
    }

    /// Whether this token refers to a global rather than a parameter or local declaration.
    pub(super) fn global(&self, name: &str, index: usize) -> bool {
        !self.parameters.contains(name) && !self.local(name, index)
    }

    /// Whether this token is hidden by a local or is itself a local declaration name.
    pub(super) fn local(&self, name: &str, index: usize) -> bool {
        self.locals.get(name).is_some_and(|locals| {
            locals
                .iter()
                .any(|local| local.declaration == index || local.visible.contains(&index))
        })
    }
}

/// Recognises built-in value types and stage-local structure type names.
fn is_type(name: &str, types: &HashSet<String>) -> bool {
    if types.contains(name) {
        return true;
    }
    matches!(name, "bool" | "int" | "uint" | "float" | "double")
        || ["vec", "ivec", "uvec", "bvec", "dvec", "mat", "dmat"]
            .iter()
            .any(|prefix| {
                name.strip_prefix(prefix).is_some_and(|rest| {
                    !rest.is_empty() && rest.bytes().all(|b| matches!(b, b'2'..=b'4' | b'x'))
                })
            })
        || super::items::is_opaque(name)
}

/// Skips complete array suffixes, retaining None for malformed ranges.
fn after_arrays(toks: &[Tok], mut index: usize) -> Option<usize> {
    while toks.get(index).is_some_and(|t| t.is("[")) {
        index = matching(toks, index, "[", "]")? + 1;
    }
    Some(index)
}

/// Finds a complete parenthesis, bracket or brace group starting at this index.
fn group_end(toks: &[Tok], index: usize) -> Option<usize> {
    match toks.get(index)?.text() {
        "(" => matching(toks, index, "(", ")"),
        "[" => matching(toks, index, "[", "]"),
        "{" => matching(toks, index, "{", "}"),
        _ => None,
    }
}

/// Finds the exclusive end of a statement, including nested control-flow substatements.
fn statement_end(toks: &[Tok], start: usize) -> usize {
    let mut position = start;
    let mut controls = Vec::new();
    loop {
        let Some(token) = toks.get(position) else {
            return toks.len();
        };
        if matches!(token.ident(), Some("if" | "for" | "while" | "switch"))
            && let Some(close) = matching(toks, position + 1, "(", ")")
        {
            controls.push(token.ident() == Some("if"));
            position = close + 1;
            continue;
        }
        if token.ident() == Some("do") {
            controls.push(false);
            position += 1;
            continue;
        }
        if token.is("{") {
            position = matching(toks, position, "{", "}").map_or(toks.len(), |end| end + 1);
        } else {
            while position < toks.len() {
                if toks[position].is(";") {
                    position += 1;
                    break;
                }
                if let Some(end) = group_end(toks, position) {
                    position = end + 1;
                } else {
                    position += 1;
                }
            }
        }
        loop {
            match controls.pop() {
                Some(true) if toks.get(position).and_then(Tok::ident) == Some("else") => {
                    position += 1;
                    break;
                }
                Some(_) => {}
                None => return position,
            }
        }
    }
}
