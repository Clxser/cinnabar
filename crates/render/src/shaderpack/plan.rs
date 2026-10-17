//! The device-independent frame plan of a loaded pack: which colour buffers exist with which
//! format, size and clear, and for each full-screen pass which side of every ping-pong
//! buffer it reads and writes.
//!
//! OptiFine/Iris flip semantics: every buffer has a main and an alternate texture and starts
//! the frame unflipped (reads main). A deferred/composite pass reads each buffer's current
//! side and draws into the other side; afterwards each buffer it wrote flips, unless
//! `flip.<program>.<buffer>=false`, and `flip.<program>.<buffer>=true` also flips a buffer
//! the pass did not write. The plan is identical every frame, and buffers that end the frame
//! flipped are copied back to main after `final`, so the next frame starts unflipped.

use shaderpack::{BufferDirectives, BufferFormat, BufferId, FlipRule, ProgramGroup, SizeValue};

/// Colour buffers a pack may use (`colortex0`–`colortex15`).
pub(crate) const MAX_COLOR_BUFFERS: u32 = 16;

/// A buffer's size relative to the screen or in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Extent {
    Relative([f32; 2]),
    Absolute([u32; 2]),
}

impl Extent {
    /// The size in pixels for a screen of `screen`, at least 1×1.
    pub(crate) fn resolve(self, screen: [u32; 2]) -> [u32; 2] {
        match self {
            Extent::Relative(scale) => {
                std::array::from_fn(|i| ((screen[i] as f32 * scale[i]).round() as u32).max(1))
            }
            Extent::Absolute(size) => size.map(|s| s.max(1)),
        }
    }
}

/// One colour buffer of the pack.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BufferPlan {
    pub index: u32,
    pub format: BufferFormat,
    pub extent: Extent,
    /// Colour both textures are cleared to at the start of each frame; `None` keeps the
    /// previous frame's contents (`colortexNClear = false`).
    pub clear: Option<[f64; 4]>,
    /// Some pass requests mipmaps, so the textures carry a full mip chain.
    pub mipmapped: bool,
    /// Some pass reads or writes the alternate texture.
    pub alternate: bool,
}

/// A program as the frame plan sees it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PlannedProgram {
    pub name: String,
    pub group: ProgramGroup,
    /// Destination buffer per fragment output location; empty for compute programs.
    pub outputs: Vec<Option<u32>>,
    /// Buffers referenced through samplers or images.
    pub reads: Vec<u32>,
    /// Buffers whose `MipmapEnabled` directive this program declares.
    pub mipmaps: Vec<u32>,
}

/// One pass of the frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PassStep {
    /// Index into the plan's programs.
    pub program: usize,
    /// Bit per buffer: samplers and images read the alternate texture.
    pub read_alt: u32,
    /// Per fragment output location: the destination buffer and whether the alternate
    /// texture is written. `final` writes the screen, so its entries are `None`.
    pub writes: Vec<Option<(u32, bool)>>,
    /// Buffers whose current side gets its mip chain regenerated before the pass.
    pub mipmaps: Vec<u32>,
}

/// The passes of one frame around the geometry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FramePlan {
    /// `prepare*` passes, before the scene.
    pub prepare: Vec<PassStep>,
    /// Bit per buffer: the side the sky, opaque geometry (or the scene copy) write (the
    /// current side after the prepare passes, as Iris's gbuffer framebuffers do).
    pub scene_alt: u32,
    /// `deferred*`, `composite*` and `final`, in order.
    pub post: Vec<PassStep>,
    /// Index in `post` of the first pass after the deferred ones; translucent geometry draws
    /// there.
    pub composite_start: usize,
    /// Bit per buffer: the side translucent geometry writes (the current side after the
    /// deferred passes).
    pub translucent_alt: u32,
    /// Buffers that end the frame flipped; their alternate texture is copied to main.
    pub swap_back: Vec<u32>,
}

/// Iris's default clear colour: `colortex1` (`gdepth`) clears to white, the rest to zero.
fn default_clear(index: u32) -> [f64; 4] {
    if index == 1 { [1.0; 4] } else { [0.0; 4] }
}

/// Parses a `ClearColor` directive value: `vec4(r, g, b, a)`, `vec3(r, g, b)` (alpha 1) or
/// a scalar (broadcast). `None` when it is not a list of 1, 3 or 4 numbers.
pub(crate) fn parse_clear_color(text: &str) -> Option<[f64; 4]> {
    let inner = text
        .trim()
        .trim_start_matches(|c: char| c.is_ascii_alphanumeric())
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')');
    let values: Vec<f64> = inner
        .split(',')
        .map(|v| v.trim().trim_end_matches(['f', 'F']).parse::<f64>().ok())
        .collect::<Option<_>>()?;
    match values.as_slice() {
        [v] => Some([*v; 4]),
        [r, g, b] => Some([*r, *g, *b, 1.0]),
        [r, g, b, a] => Some([*r, *g, *b, *a]),
        _ => None,
    }
}

fn size_extent(width: SizeValue, height: SizeValue) -> Extent {
    match (width, height) {
        (SizeValue::Absolute(w), SizeValue::Absolute(h)) => Extent::Absolute([w, h]),
        (w, h) => {
            let relative = |v| match v {
                SizeValue::Relative(r) => r,
                // A mixed pair cannot be expressed; Iris treats both as relative then.
                SizeValue::Absolute(a) => a as f32,
            };
            Extent::Relative([relative(w), relative(h)])
        }
    }
}

/// The colour buffer behind a pack sampler name (`colortex3`, `gaux1`) or image name
/// (`colorimg3`).
pub(crate) fn color_buffer_of(name: &str) -> Option<u32> {
    let id = match name.strip_prefix("colorimg") {
        Some(rest) => BufferId::parse(&format!("colortex{rest}")),
        None => BufferId::parse(name),
    };
    match id {
        Some(BufferId::Color(i)) if i < MAX_COLOR_BUFFERS => Some(i),
        _ => None,
    }
}

/// Plans the passes of a frame; `programs` are the runnable prepare/deferred/composite/final
/// programs in frame order.
pub(crate) fn plan_frame(programs: &[PlannedProgram], flips: &[FlipRule]) -> FramePlan {
    let mut flipped = 0u32;
    let step = |index: usize, program: &PlannedProgram, flipped: &mut u32| {
        let read_alt = *flipped;
        let is_final = program.group == ProgramGroup::Final;
        let writes: Vec<Option<(u32, bool)>> = program
            .outputs
            .iter()
            .map(|b| {
                b.filter(|_| !is_final)
                    .map(|b| (b, read_alt & (1 << b) == 0))
            })
            .collect();
        let mut flip = 0u32;
        for (b, _) in writes.iter().flatten() {
            flip |= 1 << b;
        }
        for rule in flips.iter().filter(|r| r.program == program.name) {
            if let Some(b) = color_buffer_of(&rule.buffer) {
                if rule.flip {
                    flip |= 1 << b;
                } else {
                    flip &= !(1 << b);
                }
            }
        }
        if !is_final {
            *flipped ^= flip;
        }
        PassStep {
            program: index,
            read_alt,
            writes,
            mipmaps: program.mipmaps.clone(),
        }
    };
    let mut prepare = vec![];
    let mut post = vec![];
    let mut translucent = None;
    for (index, program) in programs.iter().enumerate() {
        let geometry_done = !matches!(program.group, ProgramGroup::Prepare | ProgramGroup::Deferred);
        if geometry_done && translucent.is_none() {
            translucent = Some((post.len(), flipped));
        }
        let s = step(index, program, &mut flipped);
        if program.group == ProgramGroup::Prepare {
            prepare.push(s);
        } else {
            post.push(s);
        }
    }
    let (composite_start, translucent_alt) = translucent.unwrap_or((post.len(), flipped));
    // Scene writes happen between prepare and deferred: recompute the side at that point.
    let mut scene_alt = 0u32;
    let mut state = 0u32;
    for (index, program) in programs.iter().enumerate() {
        if program.group != ProgramGroup::Prepare {
            break;
        }
        step(index, program, &mut state);
        scene_alt = state;
    }
    FramePlan {
        prepare,
        scene_alt,
        post,
        composite_start,
        translucent_alt,
        swap_back: (0..MAX_COLOR_BUFFERS)
            .filter(|b| flipped & (1 << b) != 0)
            .collect(),
    }
}

/// Plans the colour buffers the passes and the `gbuffers_*` programs in `geometry` touch
/// (plus buffer 0, which receives the scene). A geometry program that samples a buffer it
/// also draws into reads the buffer's other side, so such buffers get an alternate texture.
pub(crate) fn plan_buffers(
    programs: &[PlannedProgram],
    geometry: &[PlannedProgram],
    frame: &FramePlan,
    directives: &BufferDirectives,
    sizes: &[shaderpack::BufferSize],
) -> Vec<BufferPlan> {
    let mut used = 1u32;
    let mut mipmapped = 0u32;
    let mut alternate = frame.scene_alt;
    for program in programs.iter().chain(geometry) {
        for b in program.outputs.iter().flatten().chain(&program.reads) {
            used |= 1 << b;
        }
        for b in &program.mipmaps {
            mipmapped |= 1 << b;
        }
    }
    if !geometry.is_empty() {
        alternate |= frame.translucent_alt;
    }
    for program in geometry {
        for b in program.outputs.iter().flatten() {
            if program.reads.contains(b) {
                alternate |= 1 << b;
            }
        }
    }
    for step in frame.prepare.iter().chain(&frame.post) {
        alternate |= step.read_alt;
        for (b, alt) in step.writes.iter().flatten() {
            if *alt {
                alternate |= 1 << b;
            }
        }
    }
    (0..MAX_COLOR_BUFFERS)
        .filter(|b| used & (1 << b) != 0)
        .map(|index| {
            let settings = directives.buffers.get(&BufferId::Color(index));
            let extent = sizes
                .iter()
                .rev()
                .find(|s| color_buffer_of(&s.buffer) == Some(index))
                .map_or(Extent::Relative([1.0, 1.0]), |s| {
                    size_extent(s.width, s.height)
                });
            let clear = match settings.and_then(|s| s.clear) {
                Some(false) => None,
                _ => Some(
                    settings
                        .and_then(|s| s.clear_color.as_deref())
                        .and_then(parse_clear_color)
                        .unwrap_or_else(|| default_clear(index)),
                ),
            };
            BufferPlan {
                index,
                format: directives.format(BufferId::Color(index)),
                extent,
                clear,
                mipmapped: mipmapped & (1 << index) != 0,
                alternate: alternate & (1 << index) != 0,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(name: &str, group: ProgramGroup, outputs: &[u32]) -> PlannedProgram {
        PlannedProgram {
            name: name.to_string(),
            group,
            outputs: outputs.iter().map(|b| Some(*b)).collect(),
            reads: vec![],
            mipmaps: vec![],
        }
    }

    fn flip(program: &str, buffer: &str, flip: bool) -> FlipRule {
        FlipRule {
            program: program.to_string(),
            buffer: buffer.to_string(),
            flip,
        }
    }

    #[test]
    fn written_buffers_ping_pong_and_end_flipped_buffers_swap_back() {
        let programs = [
            program("deferred", ProgramGroup::Deferred, &[0, 4]),
            program("composite", ProgramGroup::Composite, &[0]),
            program("composite1", ProgramGroup::Composite, &[4]),
            program("final", ProgramGroup::Final, &[0]),
        ];
        let plan = plan_frame(&programs, &[]);
        let deferred = &plan.post[0];
        assert_eq!(deferred.read_alt, 0);
        assert_eq!(deferred.writes, [Some((0, true)), Some((4, true))]);
        let composite = &plan.post[1];
        assert_eq!(composite.read_alt, (1 << 0) | (1 << 4));
        assert_eq!(composite.writes, [Some((0, false))]);
        let composite1 = &plan.post[2];
        assert_eq!(composite1.read_alt, 1 << 4);
        assert_eq!(composite1.writes, [Some((4, false))]);
        let fin = &plan.post[3];
        assert_eq!(fin.read_alt, 0);
        assert_eq!(fin.writes, [None]);
        assert!(plan.swap_back.is_empty());
    }

    #[test]
    fn flip_rules_suppress_and_force_flips() {
        let programs = [
            program("composite", ProgramGroup::Composite, &[3]),
            program("composite1", ProgramGroup::Composite, &[2]),
            program("final", ProgramGroup::Final, &[0]),
        ];
        let rules = [
            flip("composite", "colortex3", false),
            flip("composite1", "gaux2", true),
        ];
        let plan = plan_frame(&programs, &rules);
        assert_eq!(plan.post[0].writes, [Some((3, true))]);
        assert_eq!(plan.post[1].read_alt, 0, "colortex3 did not flip");
        assert_eq!(plan.post[2].read_alt, (1 << 2) | (1 << 5));
        assert_eq!(plan.swap_back, [2, 5]);
    }

    #[test]
    fn scene_writes_the_side_current_after_prepare() {
        let programs = [
            program("prepare", ProgramGroup::Prepare, &[0]),
            program("deferred", ProgramGroup::Deferred, &[1]),
        ];
        let plan = plan_frame(&programs, &[]);
        assert_eq!(plan.prepare.len(), 1);
        assert_eq!(plan.scene_alt, 1);
        assert_eq!(plan.post[0].read_alt, 1);
        assert_eq!(plan.swap_back, [0, 1]);
    }

    #[test]
    fn translucent_geometry_writes_the_side_current_after_deferred() {
        let programs = [
            program("deferred", ProgramGroup::Deferred, &[0, 2]),
            program("deferred1", ProgramGroup::Deferred, &[2]),
            program("composite", ProgramGroup::Composite, &[0]),
            program("final", ProgramGroup::Final, &[0]),
        ];
        let plan = plan_frame(&programs, &[]);
        assert_eq!(plan.composite_start, 2);
        assert_eq!(plan.translucent_alt, 1, "colortex0 flipped once, colortex2 twice");
        assert_eq!(plan.post[2].read_alt, plan.translucent_alt);
        let mut water = program("gbuffers_water", ProgramGroup::Gbuffers, &[0, 5]);
        water.reads = vec![5];
        let buffers = plan_buffers(
            &programs,
            &[water],
            &plan,
            &BufferDirectives::default(),
            &[],
        );
        let indices: Vec<u32> = buffers.iter().map(|b| b.index).collect();
        assert_eq!(indices, [0, 2, 5]);
        assert!(
            buffers[2].alternate,
            "a buffer a geometry program reads and writes gets a second side"
        );
        let plan = plan_frame(&programs[..2], &[]);
        assert_eq!(plan.composite_start, 2, "no composite: translucents draw last");
    }

    #[test]
    fn buffer_plan_applies_formats_sizes_clears_and_mipmaps() {
        let mut directives = BufferDirectives::default();
        for (name, value) in [
            ("colortex2Format", "RGBA16F"),
            ("colortex2Clear", "false"),
            ("colortex3ClearColor", "vec4(0.25, 0.5, 0.75, 1.0)"),
        ] {
            directives.accept(&shaderpack::ConstDirective {
                ty: "int".into(),
                name: name.into(),
                value: value.into(),
            });
        }
        let mut programs = vec![
            program("composite", ProgramGroup::Composite, &[2, 3]),
            program("final", ProgramGroup::Final, &[0]),
        ];
        programs[1].reads = vec![1, 6];
        programs[1].mipmaps = vec![2];
        let sizes = [
            shaderpack::BufferSize {
                buffer: "colortex6".into(),
                width: SizeValue::Relative(0.5),
                height: SizeValue::Relative(0.25),
            },
            shaderpack::BufferSize {
                buffer: "colortex1".into(),
                width: SizeValue::Absolute(64),
                height: SizeValue::Absolute(32),
            },
        ];
        let frame = plan_frame(&programs, &[]);
        let buffers = plan_buffers(&programs, &[], &frame, &directives, &sizes);
        let indices: Vec<u32> = buffers.iter().map(|b| b.index).collect();
        assert_eq!(indices, [0, 1, 2, 3, 6]);
        let get = |i| buffers.iter().find(|b| b.index == i).unwrap();
        assert_eq!(get(2).format, BufferFormat::Rgba16f);
        assert_eq!(get(2).clear, None);
        assert!(get(2).mipmapped && get(2).alternate);
        assert_eq!(get(3).clear, Some([0.25, 0.5, 0.75, 1.0]));
        assert_eq!(get(1).clear, Some([1.0; 4]));
        assert_eq!(get(0).format, BufferFormat::DEFAULT);
        assert!(!get(0).alternate);
        assert_eq!(get(6).extent.resolve([1920, 1080]), [960, 270]);
        assert_eq!(get(1).extent.resolve([1920, 1080]), [64, 32]);
    }

    #[test]
    fn clear_colours_and_buffer_names_parse() {
        assert_eq!(
            parse_clear_color("vec3(1.0, 0.5, 0)"),
            Some([1.0, 0.5, 0.0, 1.0])
        );
        assert_eq!(parse_clear_color("0.5"), Some([0.5; 4]));
        assert_eq!(parse_clear_color("vec2(1.0, 2.0)"), None);
        assert_eq!(color_buffer_of("gaux4"), Some(7));
        assert_eq!(color_buffer_of("colorimg12"), Some(12));
        assert_eq!(color_buffer_of("colortex16"), None);
        assert_eq!(color_buffer_of("shadowcolor0"), None);
    }
}
