//! Rewrites inside function bodies and initializers: separate texture/sampler pairs and
//! integer-backed bool uniforms.

use super::TransformError;
use super::bindings::Bindings;
use super::items::{Param, arr_len, mat_cols, matching};
use crate::reflection::{SAMPLER_NAME, SHADOW_SAMPLER_NAME};
use crate::token::{Tok, TokKind, bound_resource, id, p, resource_symbol};
use std::collections::{BTreeSet, HashMap, HashSet};

/// Bound on generated inter-stage scalar/vector locations.
const MAX_VARYING_LOCATIONS: usize = 4096;

/// Sampler name → (combined sampler type to construct, compares depth).
pub(super) type SamplerKinds = HashMap<String, (String, bool)>;

/// What body rewriting needs to know about the stage's globals.
pub(super) struct BodyContext<'a> {
    pub parameters: &'a [Param],
    pub types: &'a HashSet<String>,
    pub opaque: &'a HashSet<String>,
    pub samplers: &'a SamplerKinds,
    /// Bool uniform name → constructor (`bool` or `bvecN`) of its integer storage.
    pub bools: &'a HashMap<String, BoolUniform>,
    pub uniforms: &'a HashSet<String>,
}

/// Integer storage and array rank of a logical boolean uniform.
pub(super) struct BoolUniform {
    pub constructor: String,
    pub dimensions: usize,
}

/// A logical boolean read after consuming a prefix of its array indices.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct BoolRead {
    pub name: String,
    pub indexed: usize,
}

impl BoolRead {
    /// Returns the reserved helper symbol for this uniform and consumed index count.
    pub fn function_name(&self) -> String {
        format!(
            "__shaderpack_bool_{}_{}_{}",
            self.name.len(),
            self.name,
            self.indexed
        )
    }
}

/// Resources and logical uniform reads referenced by the stage's bodies.
#[derive(Default)]
pub(super) struct BodyUse {
    pub filtering: bool,
    pub comparison: bool,
    pub bools: BTreeSet<BoolRead>,
}

/// Pairs every texture use with the shared sampler (`sampler2D(colortex0, iris_sampler)`)
/// after resource specialization, and reads integer-backed bool uniforms as bools.
pub(super) fn rewrite_body(toks: &[Tok], cx: &BodyContext<'_>, used: &mut BodyUse) -> Vec<Tok> {
    let bindings = Bindings::new(toks, cx.parameters, cx.types);
    rewrite_range(toks, 0, &bindings, cx, used)
}

/// Rewrites a token range while keeping lexical lookup positions in the original body.
fn rewrite_range(
    toks: &[Tok],
    base: usize,
    bindings: &Bindings,
    cx: &BodyContext<'_>,
    used: &mut BodyUse,
) -> Vec<Tok> {
    let mut out = vec![];
    let mut cursor = 0;
    while cursor < toks.len() {
        let i = cursor;
        let t = &toks[i];
        cursor += 1;
        let (n, bound) = match &t.kind {
            TokKind::Ident(name) => (name, false),
            TokKind::BoundResource { original, .. } => (original, true),
            _ => {
                out.push(t.clone());
                continue;
            }
        };
        if i > 0 && toks[i - 1].is(".") {
            out.push(t.clone());
            continue;
        }
        let line = t.line;
        if let Some((ctor, shadow)) = cx
            .samplers
            .get(n)
            .filter(|_| bound || bindings.global(n, base + i))
        {
            let sampler = if *shadow {
                used.comparison = true;
                SHADOW_SAMPLER_NAME
            } else {
                used.filtering = true;
                SAMPLER_NAME
            };
            out.extend([
                id(ctor).at(line),
                p("(").at(line),
                bound_resource(n).at(line),
                p(",").at(line),
                id(sampler).at(line),
                p(")").at(line),
            ]);
            continue;
        }
        if cx.opaque.contains(n) && (bound || bindings.global(n, base + i)) {
            out.push(id(&resource_symbol(n)).at(line));
            continue;
        }
        if let Some(uniform) = cx.bools.get(n).filter(|_| bindings.global(n, base + i)) {
            let mut end = i + 1;
            let mut indices = Vec::new();
            while toks.get(end).is_some_and(|t| t.is("[")) && indices.len() < uniform.dimensions {
                let Some(close) = matching(toks, end, "[", "]") else {
                    break;
                };
                indices.push(end + 1..close);
                end = close + 1;
            }
            let read = BoolRead {
                name: n.clone(),
                indexed: indices.len(),
            };
            out.extend([id(&read.function_name()).at(line), p("(").at(line)]);
            used.bools.insert(read);
            for (index, range) in indices.into_iter().enumerate() {
                if index > 0 {
                    out.push(p(",").at(line));
                }
                // Array indices may be signed or unsigned. Each expression is evaluated
                // once, in the caller's lexical scope, before the logical value is read.
                out.extend([id("int").at(line), p("(").at(line)]);
                out.extend(rewrite_range(
                    &toks[range.clone()],
                    base + range.start,
                    bindings,
                    cx,
                    used,
                ));
                out.push(p(")").at(line));
            }
            out.push(p(")").at(line));
            cursor = end;
            continue;
        }
        if cx.uniforms.contains(n) && bindings.global(n, base + i) {
            out.extend([id(super::UNIFORM_INSTANCE).at(line), p(".").at(line)]);
        }
        out.push(t.clone());
    }
    out
}

/// Struct name → members as (type, name, array tokens).
pub(super) type Structs = HashMap<String, Vec<(String, String, Vec<Tok>)>>;

/// Flattens a matrix, array or struct value into (access path, leaf type) pairs.
pub(super) fn flatten_leaves(
    path: String,
    ty: &str,
    arr: &[Tok],
    structs: &Structs,
    line: u32,
    out: &mut Vec<(String, String)>,
) -> Result<(), TransformError> {
    let mut pending = vec![(path, ty.to_string(), arr.to_vec(), Vec::<String>::new())];
    while let Some((path, ty, arr, ancestry)) = pending.pop() {
        if pending.len() + out.len() >= MAX_VARYING_LOCATIONS {
            return Err(TransformError::new(
                "varying has too many flattened locations",
                line,
            ));
        }
        if !arr.is_empty() {
            let close = matching(&arr, 0, "[", "]")
                .ok_or_else(|| TransformError::new("unclosed varying array", line))?;
            let count = arr_len(&arr[..=close])
                .filter(|count| *count > 0 && *count as usize <= MAX_VARYING_LOCATIONS)
                .ok_or_else(|| {
                    TransformError::new(
                        format!("varying `{path}` has an unsupported array size"),
                        line,
                    )
                })?;
            for index in (0..count).rev() {
                pending.push((
                    format!("{path}[{index}]"),
                    ty.clone(),
                    arr[close + 1..].to_vec(),
                    ancestry.clone(),
                ));
            }
        } else if let Some((count, column)) = mat_cols(&ty) {
            for index in 0..count {
                out.push((format!("{path}[{index}]"), column.to_string()));
            }
        } else if let Some(members) = structs.get(&ty) {
            if ancestry.contains(&ty) {
                return Err(TransformError::new("recursive varying structure", line));
            }
            let mut ancestry = ancestry;
            ancestry.push(ty);
            for (ty, name, arr) in members.iter().rev() {
                pending.push((
                    format!("{path}.{name}"),
                    ty.clone(),
                    arr.clone(),
                    ancestry.clone(),
                ));
            }
        } else {
            out.push((path, ty));
        }
    }
    Ok(())
}
