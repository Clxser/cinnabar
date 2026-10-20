//! Per-frame evaluation of compiled [`Node`] trees, including Iris's `smooth()`
//! filter and the deterministic generator behind `random()`.

use std::f32::consts::PI;

use crate::BinaryOp;

use super::compile::{Access, Fold, Func1, Func2, Func3, Node, SmoothNode};
use super::value::{Mismatch, Value};

/// Fade time in seconds of a `smooth()` call that gives none (OptiFine default).
const DEFAULT_FADE_SECONDS: f32 = 1.0;
/// Initial state of the `random()` generator, so evaluation is reproducible.
pub(super) const RANDOM_SEED: u64 = 0x5EED_C0FF_EE15_BEEF;

/// One `smooth()` accumulator.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct SmoothState {
    /// The previous smoothed value; `None` until the call first runs.
    last: Option<Value>,
}

/// Everything a definition's evaluation reads or updates.
pub(super) struct Frame<'a> {
    /// This frame's built-in values by input slot; `None` reads as 0.
    pub(super) inputs: &'a [Option<Value>],
    /// Values of the definitions before the one being evaluated.
    pub(super) values: &'a [Value],
    pub(super) smooth: &'a mut [SmoothState],
    pub(super) rng: &'a mut u64,
    /// Seconds in the current frame, shared by every call site.
    pub(super) elapsed: f32,
}

type Eval = Result<Value, Mismatch>;

impl Node {
    /// Evaluates this node; only the taken branch of `if`, `&&` and `||` runs.
    pub(super) fn eval(&self, frame: &mut Frame<'_>) -> Eval {
        match self {
            Node::Const(value) => Ok(*value),
            Node::Input { slot, access } => {
                let input = *frame.inputs.get(*slot).ok_or(Mismatch)?;
                let Some(value) = input else {
                    return Ok(Value::Num(0.0));
                };
                match access {
                    Access::Whole => Ok(value),
                    Access::Component(index) => component(value, *index),
                    Access::Scalar => number(value).map(Value::Num),
                }
            }
            Node::Local {
                index,
                component: part,
            } => {
                let value = *frame.values.get(*index).ok_or(Mismatch)?;
                match part {
                    None => Ok(value),
                    Some(index) => component(value, *index),
                }
            }
            Node::Neg(operand) => Ok(operand.eval(frame)?.map(|n| -n)),
            Node::Not(operand) => Ok(Value::Bool(!truth(operand.eval(frame)?)?)),
            Node::Binary(op, lhs, rhs) => binary(*op, lhs, rhs, frame),
            Node::If {
                branches,
                otherwise,
            } => {
                for (condition, value) in branches {
                    if truth(condition.eval(frame)?)? {
                        return value.eval(frame);
                    }
                }
                otherwise.eval(frame)
            }
            Node::Construct(items) => {
                let mut lanes = [0.0; 4];
                for (lane, item) in lanes.iter_mut().zip(items) {
                    *lane = number(item.eval(frame)?)?;
                }
                let len = u8::try_from(items.len()).map_err(|_| Mismatch)?;
                Ok(super::value::from_lanes(Some(len), lanes))
            }
            Node::Map1(func, operand) => Ok(operand.eval(frame)?.map(|n| apply1(*func, n))),
            Node::Map2(func, args) => {
                let [a, b] = &**args;
                a.eval(frame)?
                    .zip(b.eval(frame)?, |x, y| apply2(*func, x, y))
            }
            Node::Map3(func, args) => {
                let [a, b, c] = &**args;
                let (a, b, c) = (a.eval(frame)?, b.eval(frame)?, c.eval(frame)?);
                Value::zip3(a, b, c, |x, y, z| apply3(*func, x, y, z))
            }
            Node::Fold(fold, items) => {
                let mut items = items.iter();
                let first = items.next().ok_or(Mismatch)?.eval(frame)?;
                items.try_fold(first, |acc, item| {
                    acc.zip(item.eval(frame)?, |x, y| match fold {
                        Fold::Min => x.min(y),
                        Fold::Max => x.max(y),
                    })
                })
            }
            Node::Between(args) => {
                let [x, min, max] = numbers(args, frame)?;
                Ok(Value::Bool((min..=max).contains(&x)))
            }
            Node::Equals(args) => {
                let [a, b, epsilon] = numbers(args, frame)?;
                Ok(Value::Bool((a - b).abs() <= epsilon))
            }
            Node::In(value, candidates) => {
                let value = number(value.eval(frame)?)?;
                for candidate in candidates {
                    if number(candidate.eval(frame)?)? == value {
                        return Ok(Value::Bool(true));
                    }
                }
                Ok(Value::Bool(false))
            }
            Node::Smooth(smooth) => smooth.eval(frame),
            Node::Random(range) => {
                let bounds = match range {
                    Some(args) => Some(numbers(args, frame)?),
                    None => None,
                };
                let unit = next_unit(frame.rng);
                Ok(Value::Num(match bounds {
                    Some([min, max]) => min + unit * (max - min),
                    None => unit,
                }))
            }
            Node::RandomInt(range) => {
                let bounds = match range {
                    Some(args) => {
                        let [min, max] = &**args;
                        Some([
                            min.eval(frame)?.integer().ok_or(Mismatch)?,
                            max.eval(frame)?.integer().ok_or(Mismatch)?,
                        ])
                    }
                    None => None,
                };
                let bits = next_u64(frame.rng);
                Ok(Value::Int(match bounds {
                    Some([min, max]) => {
                        let (min, max) = (i64::from(min), i64::from(max));
                        if max <= min {
                            return Err(Mismatch);
                        }
                        (min + (bits % (max - min).unsigned_abs()) as i64) as i32
                    }
                    None => bits as u32 as i32,
                }))
            }
        }
    }
}

impl SmoothNode {
    /// OptiFine `Smoother.getSmoothValue`: the first call returns its value; later calls
    /// move the previous result toward the value using the fade-up time when rising and
    /// the fade-down time when falling (defaulting to the fade-up time, itself 1 s).
    /// Vectors smooth per component; a change of shape restarts from the value.
    fn eval(&self, frame: &mut Frame<'_>) -> Eval {
        let value = self.value.eval(frame)?.to_numeric();
        let fade_up = match &self.fade_up {
            Some(fade) => number(fade.eval(frame)?)?,
            None => DEFAULT_FADE_SECONDS,
        };
        let fade_down = match &self.fade_down {
            Some(fade) => number(fade.eval(frame)?)?,
            None => fade_up,
        };
        let state = frame.smooth.get_mut(self.slot).ok_or(Mismatch)?;
        let elapsed = frame.elapsed;
        let smoothed = match state.last {
            Some(previous) if previous.lane_count() == value.lane_count() => previous
                .zip(value, |previous, target| {
                    smooth_step(previous, target, elapsed, fade_up, fade_down)
                })?,
            _ => value,
        };
        state.last = Some(smoothed);
        Ok(smoothed)
    }
}

/// Moves toward the target with Iris's half-life of one tenth of the fade time.
fn smooth_step(previous: f32, target: f32, elapsed: f32, fade_up: f32, fade_down: f32) -> f32 {
    let fade = if target > previous {
        fade_up
    } else {
        fade_down
    };
    if fade == 0.0 {
        return target;
    }
    if elapsed <= 0.0 {
        return previous;
    }
    let decay = (1.0 / (f64::from(fade * 0.1) / std::f64::consts::LN_2)) as f32;
    let fraction = 1.0 - f64::from(-decay * elapsed).exp() as f32;
    (1.0 - fraction) * previous + fraction * target
}

/// Evaluates a binary operator; `&&` and `||` short-circuit.
fn binary(op: BinaryOp, lhs: &Node, rhs: &Node, frame: &mut Frame<'_>) -> Eval {
    let l = lhs.eval(frame)?;
    match op {
        BinaryOp::And => Ok(Value::Bool(truth(l)? && truth(rhs.eval(frame)?)?)),
        BinaryOp::Or => Ok(Value::Bool(truth(l)? || truth(rhs.eval(frame)?)?)),
        BinaryOp::Mul => l.zip(rhs.eval(frame)?, |a, b| a * b),
        BinaryOp::Div => l.zip(rhs.eval(frame)?, |a, b| a / b),
        BinaryOp::Rem => l.zip(rhs.eval(frame)?, |a, b| a % b),
        BinaryOp::Add => l.zip(rhs.eval(frame)?, |a, b| a + b),
        BinaryOp::Sub => l.zip(rhs.eval(frame)?, |a, b| a - b),
        BinaryOp::Lt => compare(l, rhs.eval(frame)?, |a, b| a < b),
        BinaryOp::Le => compare(l, rhs.eval(frame)?, |a, b| a <= b),
        BinaryOp::Gt => compare(l, rhs.eval(frame)?, |a, b| a > b),
        BinaryOp::Ge => compare(l, rhs.eval(frame)?, |a, b| a >= b),
        BinaryOp::Eq => l.equals(rhs.eval(frame)?).map(Value::Bool),
        BinaryOp::Ne => l.equals(rhs.eval(frame)?).map(|equal| Value::Bool(!equal)),
    }
}

/// An ordering comparison of two scalars.
fn compare(lhs: Value, rhs: Value, op: impl Fn(f32, f32) -> bool) -> Eval {
    Ok(Value::Bool(op(number(lhs)?, number(rhs)?)))
}

/// `value` as a number, or a mismatch for a vector.
fn number(value: Value) -> Result<f32, Mismatch> {
    value.number().ok_or(Mismatch)
}

/// `value` as a condition, or a mismatch for a vector.
fn truth(value: Value) -> Result<bool, Mismatch> {
    value.truth().ok_or(Mismatch)
}

/// Component `index` of a vector, or a mismatch for a scalar or a missing component.
fn component(value: Value, index: usize) -> Eval {
    value.component(index).map(Value::Num).ok_or(Mismatch)
}

/// Evaluates `N` scalar arguments in order.
fn numbers<const N: usize>(args: &[Node; N], frame: &mut Frame<'_>) -> Result<[f32; N], Mismatch> {
    let mut out = [0.0; N];
    for (slot, arg) in out.iter_mut().zip(args) {
        *slot = number(arg.eval(frame)?)?;
    }
    Ok(out)
}

/// Applies a one-argument function to one component.
fn apply1(func: Func1, x: f32) -> f32 {
    match func {
        Func1::Sin => x.sin(),
        Func1::Cos => x.cos(),
        Func1::Tan => x.tan(),
        Func1::Asin => x.asin(),
        Func1::Acos => x.acos(),
        Func1::Atan => x.atan(),
        Func1::Radians => x * (PI / 180.0),
        Func1::Degrees => x * (180.0 / PI),
        Func1::Exp => x.exp(),
        Func1::Exp2 => x.exp2(),
        Func1::Exp10 => 10.0_f32.powf(x),
        Func1::Ln => x.ln(),
        Func1::Log2 => x.log2(),
        Func1::Log10 => x.log10(),
        Func1::Sqrt => x.sqrt(),
        Func1::Abs => x.abs(),
        // Java `Math.signum`: keeps ±0 and NaN.
        Func1::Sign => {
            if x > 0.0 {
                1.0
            } else if x < 0.0 {
                -1.0
            } else {
                x
            }
        }
        Func1::Floor => x.floor(),
        Func1::Ceil => x.ceil(),
        Func1::Frac => x - x.floor(),
        // Java `Math.round`: halves round up, NaN is 0, saturates to the int range.
        Func1::Round => {
            if x.is_nan() {
                0.0
            } else {
                (x + 0.5).floor().clamp(i32::MIN as f32, i32::MAX as f32)
            }
        }
        // Java `Math.rint`: halves round to even.
        Func1::Rint => x.round_ties_even(),
    }
}

/// Applies a two-argument function to one pair of components.
fn apply2(func: Func2, a: f32, b: f32) -> f32 {
    match func {
        Func2::Pow => a.powf(b),
        Func2::Atan2 => a.atan2(b),
        Func2::Fmod => a - b * (a / b).floor(),
        Func2::Edge => {
            if b < a {
                0.0
            } else {
                1.0
            }
        }
        Func2::LogBase => b.ln() / a.ln(),
    }
}

/// Applies a three-argument function to one triple of components.
fn apply3(func: Func3, a: f32, b: f32, c: f32) -> f32 {
    match func {
        // OptiFine `NumUtils.limit`: the lower bound wins when the bounds cross.
        Func3::Clamp => {
            if a < b {
                b
            } else if a > c {
                c
            } else {
                a
            }
        }
        Func3::Mix => a + (b - a) * c,
    }
}

/// Next output of the SplitMix64 generator.
fn next_u64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A uniformly distributed value in `[0, 1)`.
fn next_unit(state: &mut u64) -> f32 {
    const SCALE: f32 = 1.0 / (1_u32 << 24) as f32;
    (next_u64(state) >> 40) as f32 * SCALE
}
