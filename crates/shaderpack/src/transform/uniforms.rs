//! Resolves uniform types before sharing their layout across stages.

use super::StageDraft;
use crate::reflection::{LOOSE_UNIFORM_BINDING, LOOSE_UNIFORM_BLOCK, UNIFORM_GROUP};
use crate::token::{Emitter, Tok};
use naga::{ArraySize, ScalarKind, TypeInner};

/// One resolved uniform or struct field, independent of stage-local declaration names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Member {
    pub name: String,
    ty: UniformType,
}

/// A resolved value shape; array sizes and nested field types carry no source dependencies.
#[derive(Clone, Debug, PartialEq, Eq)]
enum UniformType {
    Value(String),
    Array(Box<UniformType>, u32),
    Struct(Vec<Member>),
}

/// Either an owned stage declaration or a resolved declaration imported from another stage.
enum Declaration<'a> {
    Original(&'a [Tok]),
    Imported(String),
}

/// The common block layout, with concrete types and generated struct declarations.
pub(crate) struct UniformLayout {
    members: Vec<Member>,
}

impl UniformLayout {
    /// Writes canonical types and the common block using an instance to avoid global collisions.
    pub(super) fn emit(&self, draft: &StageDraft, emitter: &mut Emitter) {
        if self.members.is_empty() {
            return;
        }
        let mut definitions = Vec::new();
        let fields: Vec<_> = self
            .members
            .iter()
            .map(
                |member| match draft.ubo.iter().find(|local| local.name == member.name) {
                    Some(local) => Declaration::Original(&local.toks),
                    None => {
                        Declaration::Imported(member.ty.declaration(&member.name, &mut definitions))
                    }
                },
            )
            .collect();
        for definition in definitions {
            emitter.raw(&definition);
        }
        emitter.raw(&format!("layout(std140, set = {UNIFORM_GROUP}, binding = {LOOSE_UNIFORM_BINDING}) uniform {LOOSE_UNIFORM_BLOCK} {{\n"));
        for field in fields {
            match field {
                Declaration::Original(tokens) => emitter.tokens(tokens),
                Declaration::Imported(text) => emitter.raw(&format!("{text};\n")),
            }
        }
        emitter.raw(&format!("}} {};\n", super::UNIFORM_INSTANCE));
    }
}

impl UniformType {
    /// Builds a portable value shape from naga's resolved type graph.
    fn resolve(module: &naga::Module, handle: naga::Handle<naga::Type>) -> Result<Self, String> {
        Ok(match &module.types[handle].inner {
            TypeInner::Scalar(s) => Self::Value(scalar(*s)?.to_string()),
            TypeInner::Vector { size, scalar: s } => {
                let prefix = match scalar(*s)? {
                    "float" => "",
                    "double" => "d",
                    "int" => "i",
                    "uint" => "u",
                    "bool" => "b",
                    other => return Err(format!("unsupported uniform vector scalar {other}")),
                };
                Self::Value(format!("{prefix}vec{}", *size as u8))
            }
            TypeInner::Matrix {
                columns,
                rows,
                scalar: s,
            } => {
                let prefix = if scalar(*s)? == "double" { "d" } else { "" };
                Self::Value(format!("{prefix}mat{}x{}", *columns as u8, *rows as u8))
            }
            TypeInner::Array {
                base,
                size: ArraySize::Constant(size),
                ..
            } => Self::Array(Box::new(Self::resolve(module, *base)?), size.get()),
            TypeInner::Struct { members, .. } => Self::Struct(
                members
                    .iter()
                    .map(|member| {
                        Ok(Member {
                            name: member.name.clone().ok_or("unnamed uniform field")?,
                            ty: Self::resolve(module, member.ty)?,
                        })
                    })
                    .collect::<Result<_, String>>()?,
            ),
            other => return Err(format!("unsupported loose uniform type {other:?}")),
        })
    }

    /// Declares a field with literal array extents, emitting any needed private struct types.
    fn declaration(&self, name: &str, definitions: &mut Vec<String>) -> String {
        match self {
            Self::Value(ty) => format!("{ty} {name}"),
            Self::Array(base, size) => base.declaration(&format!("{name}[{size}]"), definitions),
            Self::Struct(members) => {
                let fields: Vec<_> = members
                    .iter()
                    .map(|member| member.ty.declaration(&member.name, definitions))
                    .collect();
                let ty = format!("__shaderpack_uniform_type_{}", definitions.len());
                definitions.push(format!("struct {ty} {{ {}; }};\n", fields.join("; ")));
                format!("{ty} {name}")
            }
        }
    }
}

/// Maps supported naga scalar kinds to GLSL value type names.
fn scalar(scalar: naga::Scalar) -> Result<&'static str, String> {
    match (scalar.kind, scalar.width) {
        (ScalarKind::Float, 4) => Ok("float"),
        (ScalarKind::Float, 8) => Ok("double"),
        (ScalarKind::Sint, 4) => Ok("int"),
        (ScalarKind::Uint, 4) => Ok("uint"),
        (ScalarKind::Bool, _) => Ok("bool"),
        _ => Err(format!("unsupported uniform scalar {scalar:?}")),
    }
}

/// Resolves each stage's own declarations, then merges compatible concrete uniform shapes.
pub(crate) fn merge_uniforms<'a>(
    drafts: impl IntoIterator<Item = &'a StageDraft>,
) -> Result<UniformLayout, String> {
    let mut members: Vec<Member> = Vec::new();
    for draft in drafts {
        if draft.ubo.is_empty() {
            continue;
        }
        let mut emitter = Emitter::default();
        emitter.raw(&draft.header);
        emitter.tokens(&draft.hoisted);
        if draft.ubo.iter().any(|member| member.name == "iris_Fog") {
            emitter.raw(super::assemble::FOG_STRUCT);
        }
        emitter.raw(&format!("layout(std140, set = {UNIFORM_GROUP}, binding = {LOOSE_UNIFORM_BINDING}) uniform {LOOSE_UNIFORM_BLOCK} {{\n"));
        for member in &draft.ubo {
            emitter.tokens(&member.toks);
        }
        emitter.raw("};\nvoid main() {}\n");
        let (source, _) = emitter.finish();
        let module = naga::front::glsl::Frontend::default()
            .parse(
                &naga::front::glsl::Options::from(naga::ShaderStage::Vertex),
                &source,
            )
            .map_err(|error| {
                format!(
                    "cannot resolve uniform declarations: {}",
                    error.emit_to_string(&source)
                )
            })?;
        let block = module
            .global_variables
            .iter()
            .find_map(|(_, variable)| {
                (variable.binding.as_ref().is_some_and(|b| {
                    b.group == UNIFORM_GROUP && b.binding == LOOSE_UNIFORM_BINDING
                }))
                .then_some(variable.ty)
            })
            .ok_or("uniform block was not parsed")?;
        let UniformType::Struct(fields) = UniformType::resolve(&module, block)? else {
            return Err("uniform block has no fields".into());
        };
        for member in fields {
            match members.iter().find(|existing| existing.name == member.name) {
                Some(existing) if existing.ty != member.ty => {
                    return Err(format!(
                        "uniform `{}` has different resolved types in different stages",
                        member.name
                    ));
                }
                Some(_) => {}
                None => members.push(member),
            }
        }
    }
    Ok(UniformLayout { members })
}
