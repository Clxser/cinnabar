//! What each sampler and image name of a program binds, and the pack-wide textures that do
//! not depend on the view: `noisetex`, custom textures from `shaders.properties`, the
//! shadow maps (cleared until the geometry adapter renders them) and placeholders for names
//! the host cannot supply.

use super::effects::resources::VanillaTextures;
use super::loader::GpuContext;
use super::pipelines::{FormatSupport, Kind, format_kind, storage_format, texture_format};
use super::plan::{MAX_COLOR_BUFFERS, color_buffer_of};
use shaderpack::{
    BufferFormat, BufferId, CustomImage, ImageSize, ProgramGroup, ProgramReflection, SampleType,
    ShaderPack, TextureDimension, TextureSource as PackTexture,
};
use std::collections::HashMap;
use wgpu::{TextureFormat, TextureUsages};

pub(crate) mod bindings;

/// Iris's default `noiseTextureResolution`.
const DEFAULT_NOISE_RESOLUTION: u32 = 256;
/// Iris's default `shadowMapResolution`.
const DEFAULT_SHADOW_RESOLUTION: u32 = 1024;
/// Largest custom texture side the host decodes.
const MAX_CUSTOM_TEXTURE_SIDE: u32 = 8192;

/// What a sampled texture binding reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextureSource {
    /// `colortexN`, on the side the pass reads.
    Buffer { index: u32, format: TextureFormat },
    /// `depthtexN` as floats.
    Depth { index: usize, format: TextureFormat },
    /// `depthtexN` for the comparison sampler.
    DepthCompare(usize),
    /// A custom image read through its sampler name.
    Image { index: usize, format: TextureFormat },
    /// A pack-wide texture.
    Constant { index: usize, format: TextureFormat },
    /// The block atlas Cinnabar builds for geometry programs (`gtexture`, `normals`,
    /// `specular`).
    Atlas(AtlasLayer),
}

/// One of the block atlases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AtlasLayer {
    Color,
    Normals,
    Specular,
}

/// The format every atlas view has.
pub(crate) const ATLAS_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;

impl TextureSource {
    pub(crate) fn format(&self) -> TextureFormat {
        match *self {
            TextureSource::Buffer { format, .. }
            | TextureSource::Depth { format, .. }
            | TextureSource::Image { format, .. }
            | TextureSource::Constant { format, .. } => format,
            TextureSource::DepthCompare(_) => TextureFormat::Depth32Float,
            TextureSource::Atlas(_) => ATLAS_FORMAT,
        }
    }
}

/// The atlas layer a geometry program's sampler name reads, as Iris binds them.
fn atlas_layer(name: &str) -> Option<AtlasLayer> {
    match name {
        "gtexture" | "texture" | "tex" => Some(AtlasLayer::Color),
        "normals" => Some(AtlasLayer::Normals),
        "specular" => Some(AtlasLayer::Specular),
        _ => None,
    }
}

/// What a storage-image binding writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImageSource {
    /// `colorimgN`: the buffer's current side.
    Buffer(u32),
    /// A custom image.
    Image(usize),
    /// A placeholder the program may write but nothing reads.
    Constant(usize),
}

/// A program's bindings, parallel to its reflection's textures and storage images.
pub(crate) struct ProgramBindings {
    pub textures: Vec<TextureSource>,
    pub images: Vec<ImageSource>,
}

pub(crate) struct ConstantTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}

/// The shadow maps some program binds, as indices of pack-wide textures. The shadow pass
/// renders into them each frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ShadowSlots {
    /// Side of every shadow map.
    pub resolution: u32,
    /// `shadowtex0`/`shadowtex1` read through the comparison sampler (depth formats).
    pub depth: [Option<usize>; 2],
    /// `shadowtex0`/`shadowtex1` read as floats.
    pub float: [Option<usize>; 2],
    /// `shadowcolorN` by N, with its format.
    pub color: Vec<(u32, usize, TextureFormat)>,
}

/// Pack-wide textures, indexed by [`TextureSource::Constant`] and [`ImageSource::Constant`].
#[derive(Default)]
pub(crate) struct ConstantTextures {
    pub textures: Vec<ConstantTexture>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Key {
    Noise,
    Custom(String, String, TextureDimension),
    ShadowDepth(u32),
    ShadowFloat(u32),
    ShadowColor(u32),
    Lightmap,
    Placeholder(TextureFormat, TextureDimension, bool),
}

/// Creates pack-wide textures on demand while programs are resolved.
pub(crate) struct ConstantBuilder<'a> {
    gpu: &'a GpuContext,
    pack: &'a ShaderPack,
    translation: &'a shaderpack::PackTranslation,
    /// What `minecraft:` resource references resolve to.
    vanilla: &'a VanillaTextures,
    support: FormatSupport,
    shadow_resolution: u32,
    noise_resolution: u32,
    keys: HashMap<Key, (usize, TextureFormat)>,
    out: ConstantTextures,
    clears: Vec<(usize, wgpu::LoadOp<f32>, bool)>,
}

fn const_value<'a>(translation: &'a shaderpack::PackTranslation, name: &str) -> Option<&'a str> {
    translation
        .programs
        .iter()
        .filter(|p| p.enabled)
        .flat_map(|p| &p.consts)
        .rev()
        .find(|c| c.name == name)
        .map(|c| c.value.as_str())
}

impl<'a> ConstantBuilder<'a> {
    pub(crate) fn new(
        gpu: &'a GpuContext,
        pack: &'a ShaderPack,
        translation: &'a shaderpack::PackTranslation,
        vanilla: &'a VanillaTextures,
        support: FormatSupport,
    ) -> Self {
        let number = |name: &str, default: u32| {
            const_value(translation, name)
                .and_then(|v| v.trim().trim_end_matches(['f', 'F']).parse::<f32>().ok())
                .map_or(default, |v| (v as u32).clamp(1, 16384))
        };
        Self {
            gpu,
            pack,
            translation,
            vanilla,
            support,
            shadow_resolution: number("shadowMapResolution", DEFAULT_SHADOW_RESOLUTION),
            noise_resolution: number("noiseTextureResolution", DEFAULT_NOISE_RESOLUTION),
            keys: HashMap::new(),
            out: ConstantTextures::default(),
            clears: vec![],
        }
    }

    fn push(
        &mut self,
        key: Key,
        texture: wgpu::Texture,
        view: wgpu::TextureView,
    ) -> (usize, TextureFormat) {
        let format = texture.format();
        let index = self.out.textures.len();
        self.out.textures.push(ConstantTexture { texture, view });
        self.keys.insert(key, (index, format));
        (index, format)
    }

    fn texture(
        &self,
        label: &str,
        format: TextureFormat,
        size: [u32; 3],
        dimension: TextureDimension,
        usage: TextureUsages,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let (wgpu_dimension, layers, view_dimension) = match dimension {
            TextureDimension::D1 => (
                wgpu::TextureDimension::D1,
                1,
                wgpu::TextureViewDimension::D1,
            ),
            TextureDimension::D2 => (
                wgpu::TextureDimension::D2,
                1,
                wgpu::TextureViewDimension::D2,
            ),
            TextureDimension::D2Array => (
                wgpu::TextureDimension::D2,
                1,
                wgpu::TextureViewDimension::D2Array,
            ),
            TextureDimension::D3 => (
                wgpu::TextureDimension::D3,
                size[2],
                wgpu::TextureViewDimension::D3,
            ),
            TextureDimension::Cube => (
                wgpu::TextureDimension::D2,
                6,
                wgpu::TextureViewDimension::Cube,
            ),
            TextureDimension::CubeArray => (
                wgpu::TextureDimension::D2,
                6,
                wgpu::TextureViewDimension::CubeArray,
            ),
        };
        let texture = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size[0].max(1),
                height: if wgpu_dimension == wgpu::TextureDimension::D1 {
                    1
                } else {
                    size[1].max(1)
                },
                depth_or_array_layers: layers.max(1),
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu_dimension,
            format,
            usage,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(view_dimension),
            ..Default::default()
        });
        (texture, view)
    }

    /// A zero (depth: 1.0) texture of `format` and shape, shared by every unknown binding of
    /// the same type.
    pub(crate) fn placeholder(
        &mut self,
        format: TextureFormat,
        dimension: TextureDimension,
        storage: bool,
    ) -> (usize, TextureFormat) {
        let key = Key::Placeholder(format, dimension, storage);
        if let Some(found) = self.keys.get(&key) {
            return *found;
        }
        let depth = format.is_depth_stencil_format();
        let dimension = if depth {
            TextureDimension::D2
        } else {
            dimension
        };
        let mut usage = TextureUsages::TEXTURE_BINDING;
        if storage {
            usage |= TextureUsages::STORAGE_BINDING;
        }
        if depth {
            usage |= TextureUsages::RENDER_ATTACHMENT;
        }
        let (texture, view) = self.texture(
            "shader pack placeholder",
            format,
            [1, 1, 1],
            dimension,
            usage,
        );
        let found = self.push(key, texture, view);
        if depth {
            self.clears.push((found.0, wgpu::LoadOp::Clear(1.0), true));
        }
        found
    }

    fn placeholder_for(
        &mut self,
        sample: SampleType,
        dimension: TextureDimension,
    ) -> TextureSource {
        let format = match sample {
            SampleType::Float => TextureFormat::Rgba8Unorm,
            SampleType::Sint => TextureFormat::Rgba8Sint,
            SampleType::Uint => TextureFormat::Rgba8Uint,
            SampleType::Depth => TextureFormat::Depth32Float,
        };
        let (index, format) = self.placeholder(format, dimension, false);
        TextureSource::Constant { index, format }
    }

    fn noise(&mut self) -> (usize, TextureFormat) {
        if let Some(found) = self.keys.get(&Key::Noise) {
            return *found;
        }
        let file = self
            .translation
            .properties
            .custom_textures
            .iter()
            .rev()
            .find(|t| t.stage == "noise")
            .and_then(|t| self.decode(&t.source));
        let (texture, view) = match file {
            Some(decoded) => decoded,
            None => {
                let side = self.noise_resolution;
                let (texture, view) = self.texture(
                    "shader pack noisetex",
                    TextureFormat::Rgba8Unorm,
                    [side, side, 1],
                    TextureDimension::D2,
                    TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
                );
                let pixels = generated_noise(side);
                self.upload(&texture, &pixels, 4 * side, [side, side, 1]);
                (texture, view)
            }
        };
        self.push(Key::Noise, texture, view)
    }

    fn upload(&self, texture: &wgpu::Texture, bytes: &[u8], bytes_per_row: u32, size: [u32; 3]) {
        self.gpu.queue.write_texture(
            texture.as_image_copy(),
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(size[1]),
            },
            wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: size[2],
            },
        );
    }

    /// Decodes a PNG, raw or vanilla resource custom texture; `None` (logged) when it cannot
    /// be read.
    fn decode(&self, source: &PackTexture) -> Option<(wgpu::Texture, wgpu::TextureView)> {
        let result = match source {
            PackTexture::File(path) => self.decode_png(path),
            PackTexture::Raw { path, spec } => self.decode_raw(path, spec),
            PackTexture::Resource(location) => self.vanilla.resolve(location).and_then(|image| {
                self.rgba_texture(location, image.width, image.height, &image.rgba8)
            }),
        };
        result
            .inspect_err(|error| {
                bevy::log::warn!(target: "cinnabar::shaderpack", "custom texture skipped: {error}");
            })
            .ok()
    }

    fn decode_png(&self, path: &str) -> Result<(wgpu::Texture, wgpu::TextureView), String> {
        let bytes = self
            .pack
            .source()
            .read_bytes(path)
            .map_err(|e| format!("{path}: {e}"))?;
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().map_err(|e| format!("{path}: {e}"))?;
        let mut pixels = vec![0; reader.output_buffer_size().ok_or("png too large")?];
        let info = reader
            .next_frame(&mut pixels)
            .map_err(|e| format!("{path}: {e}"))?;
        let channels = info.color_type.samples();
        let rgba: Vec<u8> = pixels[..info.buffer_size()]
            .chunks_exact(channels)
            .flat_map(|p| match *p {
                [v] => [v, v, v, 255],
                [v, a] => [v, v, v, a],
                [r, g, b] => [r, g, b, 255],
                [r, g, b, a] => [r, g, b, a],
                _ => [0, 0, 0, 255],
            })
            .collect();
        self.rgba_texture(path, info.width, info.height, &rgba)
    }

    /// A texture of top-row-first RGBA8 `rgba`.
    fn rgba_texture(
        &self,
        label: &str,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(wgpu::Texture, wgpu::TextureView), String> {
        if width > MAX_CUSTOM_TEXTURE_SIDE || height > MAX_CUSTOM_TEXTURE_SIDE {
            return Err(format!(
                "{label}: {width}x{height} exceeds the texture limit"
            ));
        }
        let row = 4 * width as usize;
        if row == 0 || rgba.len() != row * height as usize {
            return Err(format!(
                "{label}: {width}x{height} does not match its pixels"
            ));
        }
        // Images are stored bottom-up in pack textures, as OpenGL uploads them.
        let flipped: Vec<u8> = rgba.chunks_exact(row).rev().flatten().copied().collect();
        let (texture, view) = self.texture(
            label,
            TextureFormat::Rgba8Unorm,
            [width, height, 1],
            TextureDimension::D2,
            TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        );
        self.upload(&texture, &flipped, 4 * width, [width, height, 1]);
        Ok((texture, view))
    }

    /// `<TYPE> <internalFormat> <width> [<height> [<depth>]] <pixelFormat> <pixelType>`.
    fn decode_raw(
        &self,
        path: &str,
        spec: &[String],
    ) -> Result<(wgpu::Texture, wgpu::TextureView), String> {
        let raw = RawSpec::parse(spec)
            .ok_or_else(|| format!("{path}: unsupported raw texture {spec:?}"))?;
        let bytes = self
            .pack
            .source()
            .read_bytes(path)
            .map_err(|e| format!("{path}: {e}"))?;
        let format = texture_format(raw.internal, self.support);
        let pixels = raw
            .convert(&bytes, format)
            .ok_or_else(|| format!("{path}: data does not match {spec:?}"))?;
        let block = format.block_copy_size(None).unwrap_or(4);
        let (texture, view) = self.texture(
            path,
            format,
            raw.size,
            raw.dimension,
            TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        );
        let size = if raw.dimension == TextureDimension::D1 {
            [raw.size[0], 1, 1]
        } else {
            raw.size
        };
        self.upload(&texture, &pixels, block * size[0], size);
        Ok((texture, view))
    }

    /// The stage's custom texture for sampler `name` declared with `dimension`: OptiFine binds
    /// each shape on its own target of the unit, so `texture.composite.colortex0 = x.dat
    /// TEXTURE_3D` replaces only a `sampler3D colortex0` and a `sampler2D colortex0` still
    /// reads the buffer.
    fn custom(
        &mut self,
        stage: &str,
        name: &str,
        dimension: TextureDimension,
    ) -> Option<(usize, TextureFormat)> {
        let key = Key::Custom(stage.to_string(), name.to_string(), dimension);
        if let Some(found) = self.keys.get(&key) {
            return Some(*found);
        }
        let source = self
            .translation
            .properties
            .custom_textures
            .iter()
            .rev()
            .find(|t| t.stage == stage && t.uniform() == name && t.source.dimension() == dimension)?
            .source
            .clone();
        let (texture, view) = self.decode(&source)?;
        Some(self.push(key, texture, view))
    }

    fn shadow(&mut self, index: u32, sample: SampleType) -> (usize, TextureFormat) {
        let key = if sample == SampleType::Depth {
            Key::ShadowDepth(index)
        } else {
            Key::ShadowFloat(index)
        };
        if let Some(found) = self.keys.get(&key) {
            return *found;
        }
        let side = self.shadow_resolution;
        let format = if sample == SampleType::Depth {
            TextureFormat::Depth32Float
        } else if self.support.float32_filterable {
            TextureFormat::R32Float
        } else {
            TextureFormat::R16Float
        };
        let (texture, view) = self.texture(
            "shader pack shadowtex",
            format,
            [side, side, 1],
            TextureDimension::D2,
            TextureUsages::TEXTURE_BINDING
                | TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::COPY_DST,
        );
        let found = self.push(key, texture, view);
        self.clears.push((
            found.0,
            wgpu::LoadOp::Clear(1.0),
            sample == SampleType::Depth,
        ));
        found
    }

    /// `shadowcolorN`, created on first use; the shadow pass draws into it.
    pub(crate) fn shadow_color(&mut self, index: u32) -> (usize, TextureFormat) {
        if let Some(found) = self.keys.get(&Key::ShadowColor(index)) {
            return *found;
        }
        let side = self.shadow_resolution;
        let format = texture_format(
            self.translation
                .buffers
                .format(BufferId::ShadowColor(index)),
            self.support,
        );
        let (texture, view) = self.texture(
            "shader pack shadowcolor",
            format,
            [side, side, 1],
            TextureDimension::D2,
            TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
        );
        let found = self.push(Key::ShadowColor(index), texture, view);
        self.clears.push((found.0, wgpu::LoadOp::Clear(1.0), false));
        found
    }

    /// The shadow maps created so far: call after every program is resolved.
    pub(crate) fn shadow_slots(&self) -> ShadowSlots {
        let found = |key: Key| self.keys.get(&key).map(|(index, _)| *index);
        let mut color: Vec<_> = self
            .keys
            .iter()
            .filter_map(|(key, (index, format))| match key {
                Key::ShadowColor(i) => Some((*i, *index, *format)),
                _ => None,
            })
            .collect();
        color.sort_unstable_by_key(|(i, ..)| *i);
        ShadowSlots {
            resolution: self.shadow_resolution,
            depth: [0, 1].map(|i| found(Key::ShadowDepth(i))),
            float: [0, 1].map(|i| found(Key::ShadowFloat(i))),
            color,
        }
    }

    /// The 16×16 `lightmap` geometry programs sample with `gl_TextureMatrix[1]` coordinates,
    /// block light along U and sky light along V. It starts as the daylight table;
    /// [`super::lightmap::update_lightmap`] keeps it equal to the world lightmap.
    fn lightmap(&mut self) -> (usize, TextureFormat) {
        if let Some(found) = self.keys.get(&Key::Lightmap) {
            return *found;
        }
        let side = super::lightmap::LIGHTMAP_SIDE;
        let (texture, view) = self.texture(
            "shader pack lightmap",
            TextureFormat::Rgba8Unorm,
            [side, side, 1],
            TextureDimension::D2,
            TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        );
        let texels =
            super::lightmap::lightmap_texels(Some(render_model::LightmapInputs::default()));
        self.upload(&texture, &texels, side * 4, [side, side, 1]);
        self.push(Key::Lightmap, texture, view)
    }

    /// The `lightmap` texture, when some program samples it.
    pub(crate) fn lightmap_slot(&self) -> Option<usize> {
        self.keys.get(&Key::Lightmap).map(|(index, _)| *index)
    }

    /// Whether pack-wide texture `index` has the shape a sampler of `dimension` reads.
    fn has_shape(&self, index: usize, dimension: TextureDimension) -> bool {
        self.out.textures[index].texture.dimension() == storage_shape(dimension)
    }

    /// Clears the textures that start at a value other than zero and returns them.
    pub(crate) fn finish(self) -> ConstantTextures {
        if !self.clears.is_empty() {
            let mut encoder =
                self.gpu
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("shader pack constant clears"),
                    });
            for (index, value, depth) in &self.clears {
                let view = &self.out.textures[*index].view;
                let wgpu::LoadOp::Clear(value) = *value else {
                    continue;
                };
                let colour = [Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::from(value),
                            g: f64::from(value),
                            b: f64::from(value),
                            a: f64::from(value),
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })];
                let depth_attachment = depth.then(|| wgpu::RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(value),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                });
                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("shader pack constant clear"),
                    color_attachments: if *depth { &[] } else { &colour },
                    depth_stencil_attachment: depth_attachment,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
            }
            self.gpu.queue.submit([encoder.finish()]);
        }
        self.out
    }
}

/// Deterministic white noise standing in for Minecraft's generated `noisetex`.
fn generated_noise(side: u32) -> Vec<u8> {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    (0..side as usize * side as usize * 4)
        .map(|i| {
            if i % 4 == 3 {
                return 255;
            }
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 56) as u8
        })
        .collect()
}

/// A parsed raw texture specification.
#[derive(Clone, Debug, PartialEq)]
struct RawSpec {
    dimension: TextureDimension,
    internal: BufferFormat,
    size: [u32; 3],
    channels: usize,
    pixel: PixelType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PixelType {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    F16,
    F32,
}

impl PixelType {
    fn bytes(self) -> usize {
        match self {
            PixelType::U8 | PixelType::I8 => 1,
            PixelType::U16 | PixelType::I16 | PixelType::F16 => 2,
            PixelType::U32 | PixelType::I32 | PixelType::F32 => 4,
        }
    }

    /// Component `bytes` as f64: normalized for 8/16-bit unsigned and signed types unless
    /// `integer`, raw otherwise.
    fn read(self, bytes: &[u8], integer: bool) -> f64 {
        let norm = |v: f64, max: f64| if integer { v } else { v / max };
        match self {
            PixelType::U8 => norm(f64::from(bytes[0]), 255.0),
            PixelType::I8 => norm(f64::from(bytes[0] as i8), 127.0),
            PixelType::U16 => norm(f64::from(u16::from_le_bytes([bytes[0], bytes[1]])), 65535.0),
            PixelType::I16 => norm(f64::from(i16::from_le_bytes([bytes[0], bytes[1]])), 32767.0),
            PixelType::U32 => {
                f64::from(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            }
            PixelType::I32 => {
                f64::from(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            }
            PixelType::F16 => f64::from(half_to_f32(u16::from_le_bytes([bytes[0], bytes[1]]))),
            PixelType::F32 => {
                f64::from(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            }
        }
    }
}

fn half_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from((h >> 10) & 0x1f);
    let mantissa = f32::from(h & 0x3ff);
    sign * match exponent {
        0 => mantissa * 2f32.powi(-24),
        31 if mantissa == 0.0 => f32::INFINITY,
        31 => f32::NAN,
        e => (1.0 + mantissa / 1024.0) * 2f32.powi(e - 15),
    }
}

pub(crate) fn f32_to_half(v: f32) -> u16 {
    let bits = v.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    let mantissa = bits & 0x7f_ffff;
    if v.is_nan() {
        return sign | 0x7e00;
    }
    if exponent >= 31 {
        return sign | 0x7c00;
    }
    if exponent <= 0 {
        if exponent < -10 {
            return sign;
        }
        let m = (mantissa | 0x80_0000) >> (1 - exponent);
        return sign | (m >> 13) as u16;
    }
    sign | ((exponent as u16) << 10) | (mantissa >> 13) as u16
}

impl RawSpec {
    fn parse(spec: &[String]) -> Option<RawSpec> {
        let dimension = match spec.first()?.as_str() {
            "TEXTURE_1D" => TextureDimension::D1,
            "TEXTURE_2D" | "TEXTURE_RECTANGLE" => TextureDimension::D2,
            "TEXTURE_3D" => TextureDimension::D3,
            _ => return None,
        };
        let dims = match dimension {
            TextureDimension::D1 => 1,
            TextureDimension::D3 => 3,
            _ => 2,
        };
        let internal = BufferFormat::parse(spec.get(1)?)?;
        let mut size = [1u32; 3];
        for (i, slot) in size.iter_mut().take(dims).enumerate() {
            *slot = spec
                .get(2 + i)?
                .parse()
                .ok()
                .filter(|v| (1..=MAX_CUSTOM_TEXTURE_SIDE).contains(v))?;
        }
        let channels = match spec.get(2 + dims)?.trim_end_matches("_INTEGER") {
            "RED" | "R" => 1,
            "RG" => 2,
            "RGB" | "BGR" => 3,
            "RGBA" | "BGRA" => 4,
            _ => return None,
        };
        let pixel = match spec.get(3 + dims)?.as_str() {
            "UNSIGNED_BYTE" => PixelType::U8,
            "BYTE" => PixelType::I8,
            "UNSIGNED_SHORT" => PixelType::U16,
            "SHORT" => PixelType::I16,
            "UNSIGNED_INT" => PixelType::U32,
            "INT" => PixelType::I32,
            "HALF_FLOAT" => PixelType::F16,
            "FLOAT" => PixelType::F32,
            _ => return None,
        };
        Some(RawSpec {
            dimension,
            internal,
            size,
            channels,
            pixel,
        })
    }

    /// Converts the file's pixels into `format`'s texel bytes; `None` if the data is short
    /// or the format is not one the converter writes.
    fn convert(&self, bytes: &[u8], format: TextureFormat) -> Option<Vec<u8>> {
        let texels = self.size.iter().map(|v| *v as usize).product::<usize>();
        let stride = self.channels * self.pixel.bytes();
        if bytes.len() < texels * stride {
            return None;
        }
        let integer = format_kind(format) != Kind::Float;
        let (channels, write): (usize, fn(f64, &mut Vec<u8>)) = match format {
            TextureFormat::R8Unorm => (1, |v, o| o.push((v.clamp(0.0, 1.0) * 255.0).round() as u8)),
            TextureFormat::Rg8Unorm => {
                (2, |v, o| o.push((v.clamp(0.0, 1.0) * 255.0).round() as u8))
            }
            TextureFormat::Rgba8Unorm => {
                (4, |v, o| o.push((v.clamp(0.0, 1.0) * 255.0).round() as u8))
            }
            TextureFormat::R16Float => (1, |v, o| o.extend(f32_to_half(v as f32).to_le_bytes())),
            TextureFormat::Rg16Float => (2, |v, o| o.extend(f32_to_half(v as f32).to_le_bytes())),
            TextureFormat::Rgba16Float => (4, |v, o| o.extend(f32_to_half(v as f32).to_le_bytes())),
            TextureFormat::R32Float => (1, |v, o| o.extend((v as f32).to_le_bytes())),
            TextureFormat::Rg32Float => (2, |v, o| o.extend((v as f32).to_le_bytes())),
            TextureFormat::Rgba32Float => (4, |v, o| o.extend((v as f32).to_le_bytes())),
            TextureFormat::R32Uint => (1, |v, o| o.extend((v.max(0.0) as u32).to_le_bytes())),
            TextureFormat::R32Sint => (1, |v, o| o.extend((v as i32).to_le_bytes())),
            _ => return None,
        };
        let mut out = Vec::with_capacity(texels * channels * 4);
        for texel in bytes.chunks_exact(stride).take(texels) {
            for c in 0..channels {
                let value = if c < self.channels {
                    let at = c * self.pixel.bytes();
                    self.pixel
                        .read(&texel[at..at + self.pixel.bytes()], integer)
                } else if c == 3 {
                    1.0
                } else {
                    0.0
                };
                write(value, &mut out);
            }
        }
        Some(out)
    }
}

/// The buffer index of `depthtexN` (with `gdepthtex` as `depthtex0`).
fn depth_index(name: &str) -> Option<usize> {
    match name {
        "depthtex0" | "gdepthtex" => Some(0),
        "depthtex1" => Some(1),
        "depthtex2" => Some(2),
        _ => None,
    }
}

/// The storage shape of a texture a sampler of `dimension` reads: arrays and cubes are 2D
/// layers.
fn storage_shape(dimension: TextureDimension) -> wgpu::TextureDimension {
    match dimension {
        TextureDimension::D1 => wgpu::TextureDimension::D1,
        TextureDimension::D3 => wgpu::TextureDimension::D3,
        TextureDimension::D2
        | TextureDimension::D2Array
        | TextureDimension::Cube
        | TextureDimension::CubeArray => wgpu::TextureDimension::D2,
    }
}

/// The texture format of each custom image: the format a program declares for it, else the
/// mapping of its `internalFormat`.
pub(crate) fn image_formats(
    images: &[CustomImage],
    reflections: &[&ProgramReflection],
    support: FormatSupport,
) -> Vec<TextureFormat> {
    images
        .iter()
        .map(|image| {
            reflections
                .iter()
                .flat_map(|r| &r.storage_images)
                .find(|i| i.name == image.name)
                .and_then(|i| storage_format(i.format))
                .or_else(|| {
                    BufferFormat::parse(&image.internal_format).map(|f| texture_format(f, support))
                })
                .unwrap_or(TextureFormat::Rgba8Unorm)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn raw_specs_parse_shape_format_and_pixel_layout() {
        let raw = RawSpec::parse(&spec(&[
            "TEXTURE_3D",
            "RGB16F",
            "4",
            "2",
            "3",
            "RGB",
            "HALF_FLOAT",
        ]))
        .unwrap();
        assert_eq!(raw.dimension, TextureDimension::D3);
        assert_eq!(raw.internal, BufferFormat::Rgb16f);
        assert_eq!(raw.size, [4, 2, 3]);
        assert_eq!((raw.channels, raw.pixel), (3, PixelType::F16));
        assert!(
            RawSpec::parse(&spec(&[
                "TEXTURE_2D",
                "RGBA8",
                "0",
                "4",
                "RGBA",
                "UNSIGNED_BYTE"
            ]))
            .is_none()
        );
        assert!(
            RawSpec::parse(&spec(&[
                "TEXTURE_CUBE",
                "RGBA8",
                "4",
                "4",
                "RGBA",
                "UNSIGNED_BYTE"
            ]))
            .is_none()
        );
    }

    #[test]
    fn raw_pixels_widen_to_the_texture_format() {
        let raw =
            RawSpec::parse(&spec(&["TEXTURE_1D", "RGB16F", "2", "RGB", "HALF_FLOAT"])).unwrap();
        let mut bytes = vec![];
        for v in [0.5f32, 1.0, -2.0, 0.25, 0.0, 3.0] {
            bytes.extend(f32_to_half(v).to_le_bytes());
        }
        let out = raw.convert(&bytes, TextureFormat::Rgba16Float).unwrap();
        let halves: Vec<f32> = out
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| half_to_f32(u16::from_le_bytes(*b)))
            .collect();
        assert_eq!(halves, [0.5, 1.0, -2.0, 1.0, 0.25, 0.0, 3.0, 1.0]);
        assert!(
            raw.convert(&bytes[..4], TextureFormat::Rgba16Float)
                .is_none()
        );
        let bytes8 = RawSpec::parse(&spec(&[
            "TEXTURE_2D",
            "RGBA8",
            "1",
            "1",
            "RED",
            "UNSIGNED_BYTE",
        ]))
        .unwrap()
        .convert(&[51], TextureFormat::Rgba8Unorm)
        .unwrap();
        assert_eq!(bytes8, [51, 0, 0, 255]);
    }

    #[test]
    fn depth_names_and_generated_noise_follow_optifine() {
        assert_eq!(depth_index("gdepthtex"), Some(0));
        assert_eq!(depth_index("depthtex3"), None);
        let noise = generated_noise(4);
        assert_eq!(noise.len(), 64);
        assert!(noise.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        assert!(noise.as_chunks::<4>().0.iter().any(|p| p[0] != noise[0]));
    }
}
