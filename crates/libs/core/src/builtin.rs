//! What built-in expressions mean (design §4.3): the engine evaluates them where a branch stands,
//! with no task call. An error fails the branch, as a task's would.
//!
//! - Arithmetic, comparisons and logic read their operands as the expression's [`ScalarType`]; the
//!   result is a value of that type (or a `bool`, for comparisons and logic). Integer arithmetic is
//!   checked: an overflow, or a division by zero, is an error rather than a wrong number.
//! - Field access, indexing and tuples take values apart and put them together as MessagePack, as
//!   they cross every boundary: a field is a slice of its struct's bytes, not decoded.
//! - `format!` writes scalars as Rust's `Display` does.
//! - An option is none if it is nil, and a some is what it holds: serde writes `Some(x)` as `x`.
//! - An enum's variant is read from its MessagePack too: serde writes a unit variant as its name,
//!   and any other as a map of its name to what it holds.

use anyhow::{Context, anyhow, bail};
use bytes::Bytes;

use crate::ir::{BuiltinOp, ScalarOp, ScalarType};
use crate::msgpack;
use crate::value::{Data, Value};

/// Evaluates `op` on `inputs`.
pub fn eval(op: &BuiltinOp, inputs: &[Value]) -> anyhow::Result<Value> {
    match op {
        BuiltinOp::Scalar { op, ty } => scalar(*op, *ty, inputs),
        BuiltinOp::Field { name } => {
            let [value] = encoded::<1>(inputs)?;
            let at = msgpack::field(&value, name)?.with_context(|| format!("no field {name:?}"))?;
            Ok(Value::encoded(value.slice(at)))
        }
        BuiltinOp::Index => {
            let [list, index] = encoded::<2>(inputs)?;
            let index: usize = Value::encoded(index).take()?;
            let at = msgpack::element(&list, index)?;
            Ok(Value::encoded(list.slice(at)))
        }
        BuiltinOp::Tuple => {
            let parts = inputs
                .iter()
                .map(Value::encode)
                .collect::<anyhow::Result<Vec<_>>>()?;
            Ok(Value::encoded(msgpack::array(&parts)))
        }
        BuiltinOp::Variant { name } => {
            let [value] = encoded::<1>(inputs)?;
            Ok(Value::new(msgpack::variant(&value)? == name))
        }
        BuiltinOp::Expect { message } => {
            let [holds] = read::<bool, 1>(ScalarOp::And, inputs)?;
            anyhow::ensure!(holds, "{message}");
            Ok(Value::new(true))
        }
        BuiltinOp::IsSome => {
            let [value] = encoded::<1>(inputs)?;
            Ok(Value::new(!msgpack::is_nil(&value)))
        }
        BuiltinOp::Unwrap => {
            let [value] = encoded::<1>(inputs)?;
            anyhow::ensure!(!msgpack::is_nil(&value), "a none has no value");
            Ok(Value::encoded(value))
        }
        BuiltinOp::Format { pieces } => {
            anyhow::ensure!(
                pieces.len() == inputs.len() + 1,
                "a format of {} pieces takes {} values, not {}",
                pieces.len(),
                pieces.len().saturating_sub(1),
                inputs.len()
            );
            let mut text = pieces[0].clone();
            for (input, piece) in inputs.iter().zip(&pieces[1..]) {
                msgpack::display(&input.encode()?, &mut text)?;
                text.push_str(piece);
            }
            Ok(Value::new(text))
        }
    }
}

/// `N` inputs, as MessagePack.
fn encoded<const N: usize>(inputs: &[Value]) -> anyhow::Result<[Bytes; N]> {
    let parts = inputs
        .iter()
        .map(Value::encode)
        .collect::<anyhow::Result<Vec<_>>>()?;
    parts
        .try_into()
        .map_err(|parts: Vec<Bytes>| anyhow!("takes {N} operands, not {}", parts.len()))
}

/// Evaluates the scalar `op` on `inputs`, read as `ty`.
fn scalar(op: ScalarOp, ty: ScalarType, inputs: &[Value]) -> anyhow::Result<Value> {
    match ty {
        ScalarType::Bool => logic(op, inputs),
        ScalarType::I32 => integer(
            op,
            inputs,
            i32::checked_add,
            i32::checked_sub,
            i32::checked_mul,
            i32::checked_div,
            i32::checked_rem,
            i32::checked_neg,
        ),
        ScalarType::I64 => integer(
            op,
            inputs,
            i64::checked_add,
            i64::checked_sub,
            i64::checked_mul,
            i64::checked_div,
            i64::checked_rem,
            i64::checked_neg,
        ),
        ScalarType::U32 => integer(
            op,
            inputs,
            u32::checked_add,
            u32::checked_sub,
            u32::checked_mul,
            u32::checked_div,
            u32::checked_rem,
            u32::checked_neg,
        ),
        ScalarType::U64 => integer(
            op,
            inputs,
            u64::checked_add,
            u64::checked_sub,
            u64::checked_mul,
            u64::checked_div,
            u64::checked_rem,
            u64::checked_neg,
        ),
        ScalarType::F32 => float::<f32>(op, inputs),
        ScalarType::F64 => float::<f64>(op, inputs),
        ScalarType::String => string(op, inputs),
    }
}

fn read<T: Data, const N: usize>(op: ScalarOp, inputs: &[Value]) -> anyhow::Result<[T; N]> {
    let values: Vec<T> = inputs
        .iter()
        .map(Value::to::<T>)
        .collect::<anyhow::Result<_>>()
        .with_context(|| format!("{op:?}: operand is not a {}", std::any::type_name::<T>()))?;
    values
        .try_into()
        .map_err(|values: Vec<T>| anyhow!("{op:?} takes {N} operands, not {}", values.len()))
}

/// A comparison of `T`s, if `op` is one.
fn compare<T: Data + PartialOrd>(op: ScalarOp, inputs: &[Value]) -> anyhow::Result<Option<Value>> {
    let ordering = |test: fn(&T, &T) -> bool| -> anyhow::Result<Option<Value>> {
        let [a, b] = read::<T, 2>(op, inputs)?;
        Ok(Some(Value::new(test(&a, &b))))
    };
    match op {
        ScalarOp::Eq => ordering(T::eq),
        ScalarOp::Ne => ordering(T::ne),
        ScalarOp::Lt => ordering(T::lt),
        ScalarOp::Le => ordering(T::le),
        ScalarOp::Gt => ordering(T::gt),
        ScalarOp::Ge => ordering(T::ge),
        _ => Ok(None),
    }
}

type Checked<T> = fn(T, T) -> Option<T>;

#[allow(clippy::too_many_arguments)]
fn integer<T: Data + PartialOrd + Copy>(
    op: ScalarOp,
    inputs: &[Value],
    add: Checked<T>,
    sub: Checked<T>,
    mul: Checked<T>,
    div: Checked<T>,
    rem: Checked<T>,
    neg: fn(T) -> Option<T>,
) -> anyhow::Result<Value> {
    if let Some(value) = compare::<T>(op, inputs)? {
        return Ok(value);
    }
    let checked = |f: Checked<T>, what: &str| -> anyhow::Result<Value> {
        let [a, b] = read::<T, 2>(op, inputs)?;
        f(a, b)
            .map(Value::new)
            .ok_or_else(|| anyhow!("{what} {}", std::any::type_name::<T>()))
    };
    match op {
        ScalarOp::Add => checked(add, "overflow adding"),
        ScalarOp::Sub => checked(sub, "overflow subtracting"),
        ScalarOp::Mul => checked(mul, "overflow multiplying"),
        ScalarOp::Div => checked(div, "division by zero or overflow dividing"),
        ScalarOp::Rem => checked(rem, "division by zero or overflow in a remainder of"),
        ScalarOp::Neg => {
            let [a] = read::<T, 1>(op, inputs)?;
            neg(a)
                .map(Value::new)
                .ok_or_else(|| anyhow!("overflow negating {}", std::any::type_name::<T>()))
        }
        _ => bail!("{op:?} is not defined on {}", std::any::type_name::<T>()),
    }
}

fn float<T>(op: ScalarOp, inputs: &[Value]) -> anyhow::Result<Value>
where
    T: Data
        + PartialOrd
        + std::ops::Add<Output = T>
        + std::ops::Sub<Output = T>
        + std::ops::Mul<Output = T>
        + std::ops::Div<Output = T>
        + std::ops::Rem<Output = T>
        + std::ops::Neg<Output = T>,
{
    if let Some(value) = compare::<T>(op, inputs)? {
        return Ok(value);
    }
    let arith = |f: fn(T, T) -> T| -> anyhow::Result<Value> {
        let [a, b] = read::<T, 2>(op, inputs)?;
        Ok(Value::new(f(a, b)))
    };
    match op {
        ScalarOp::Add => arith(T::add),
        ScalarOp::Sub => arith(T::sub),
        ScalarOp::Mul => arith(T::mul),
        ScalarOp::Div => arith(T::div),
        ScalarOp::Rem => arith(T::rem),
        ScalarOp::Neg => {
            let [a] = read::<T, 1>(op, inputs)?;
            Ok(Value::new(-a))
        }
        _ => bail!("{op:?} is not defined on {}", std::any::type_name::<T>()),
    }
}

fn logic(op: ScalarOp, inputs: &[Value]) -> anyhow::Result<Value> {
    if let Some(value) = compare::<bool>(op, inputs)? {
        return Ok(value);
    }
    match op {
        ScalarOp::And => read::<bool, 2>(op, inputs).map(|[a, b]| Value::new(a && b)),
        ScalarOp::Or => read::<bool, 2>(op, inputs).map(|[a, b]| Value::new(a || b)),
        ScalarOp::Not => read::<bool, 1>(op, inputs).map(|[a]| Value::new(!a)),
        _ => bail!("{op:?} is not defined on bool"),
    }
}

fn string(op: ScalarOp, inputs: &[Value]) -> anyhow::Result<Value> {
    if let Some(value) = compare::<String>(op, inputs)? {
        return Ok(value);
    }
    match op {
        ScalarOp::Add => read::<String, 2>(op, inputs).map(|[a, b]| Value::new(a + &b)),
        _ => bail!("{op:?} is not defined on String"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run<T: Data>(op: ScalarOp, ty: ScalarType, inputs: &[Value]) -> anyhow::Result<T> {
        eval(&BuiltinOp::Scalar { op, ty }, inputs)?.take()
    }

    #[test]
    fn integers_are_checked() {
        let (two, three) = (Value::new(2i64), Value::new(3i64));
        assert_eq!(
            run::<i64>(
                ScalarOp::Add,
                ScalarType::I64,
                &[two.clone(), three.clone()]
            )
            .unwrap(),
            5
        );
        assert_eq!(
            run::<i64>(
                ScalarOp::Rem,
                ScalarType::I64,
                &[three.clone(), two.clone()]
            )
            .unwrap(),
            1
        );
        assert!(run::<bool>(ScalarOp::Lt, ScalarType::I64, &[two.clone(), three]).unwrap());
        let max = Value::new(i32::MAX);
        let overflow = run::<i32>(ScalarOp::Add, ScalarType::I32, &[max, Value::new(1i32)]);
        assert!(overflow.unwrap_err().to_string().contains("overflow"));
        let zero = run::<i64>(ScalarOp::Div, ScalarType::I64, &[two, Value::new(0i64)]);
        assert!(zero.unwrap_err().to_string().contains("division by zero"));
        assert!(
            run::<u32>(
                ScalarOp::Sub,
                ScalarType::U32,
                &[Value::new(1u32), Value::new(2u32)]
            )
            .is_err()
        );
    }

    #[test]
    fn operands_are_read_as_the_expression_s_type_whatever_their_form() {
        // A literal or a task's output arrives encoded; 10 encodes as a small integer, and reads
        // as an i64 or an f64 alike.
        let ten = Value::encoded(rmp_serde::to_vec(&10).unwrap());
        assert_eq!(
            run::<i64>(
                ScalarOp::Mul,
                ScalarType::I64,
                &[ten.clone(), Value::new(3i64)]
            )
            .unwrap(),
            30
        );
        assert_eq!(
            run::<f64>(ScalarOp::Div, ScalarType::F64, &[ten, Value::new(4.0f64)]).unwrap(),
            2.5
        );
    }

    #[test]
    fn logic_and_strings() {
        let (t, f) = (Value::new(true), Value::new(false));
        assert!(!run::<bool>(ScalarOp::And, ScalarType::Bool, &[t.clone(), f.clone()]).unwrap());
        assert!(run::<bool>(ScalarOp::Or, ScalarType::Bool, &[t.clone(), f]).unwrap());
        assert!(!run::<bool>(ScalarOp::Not, ScalarType::Bool, &[t]).unwrap());
        let (a, b) = (Value::new("na".to_string()), Value::new("mu".to_string()));
        assert_eq!(
            run::<String>(ScalarOp::Add, ScalarType::String, &[a.clone(), b.clone()]).unwrap(),
            "namu"
        );
        assert!(run::<bool>(ScalarOp::Gt, ScalarType::String, &[a.clone(), b.clone()]).unwrap());
        assert!(scalar(ScalarOp::Sub, ScalarType::String, &[a, b]).is_err());
    }

    #[test]
    fn an_enum_is_told_by_its_variant_and_holds_its_fields() {
        #[derive(serde::Serialize)]
        enum Fetched {
            Gone,
            Moved(String),
            Limited { retry_after: u64 },
        }
        let encode = |fetched: &Fetched| Value::encoded(rmp_serde::to_vec_named(fetched).unwrap());
        let is = |value: &Value, name: &str| -> bool {
            let op = BuiltinOp::Variant { name: name.into() };
            eval(&op, std::slice::from_ref(value))
                .unwrap()
                .take()
                .unwrap()
        };
        let field = |value: Value, name: &str| {
            eval(&BuiltinOp::Field { name: name.into() }, &[value]).unwrap()
        };
        let gone = encode(&Fetched::Gone);
        assert!(is(&gone, "Gone") && !is(&gone, "Moved"));
        let moved = encode(&Fetched::Moved("/new".into()));
        assert!(is(&moved, "Moved"));
        let to: String = field(moved, "Moved").take().unwrap();
        assert_eq!(to, "/new");
        let limited = encode(&Fetched::Limited { retry_after: 3 });
        assert!(is(&limited, "Limited"));
        let after: u64 = field(field(limited.clone(), "Limited"), "retry_after")
            .take()
            .unwrap();
        assert_eq!(after, 3);
        // What no arm matches fails.
        let expect = BuiltinOp::Expect {
            message: "no arm".into(),
        };
        let missed = eval(&expect, &[Value::new(false)]).unwrap_err().to_string();
        assert_eq!(missed, "no arm");
        assert!(eval(&expect, &[Value::new(true)]).is_ok());
        assert!(is(&limited, "Limited") && !is(&gone, "Limited"));
    }

    #[test]
    fn an_option_is_some_or_none_and_a_some_is_what_it_holds() {
        let encode = |option: Option<&str>| Value::encoded(rmp_serde::to_vec(&option).unwrap());
        let some = encode(Some("page"));
        let none = encode(None);
        let is_some = |value: &Value| -> bool {
            eval(&BuiltinOp::IsSome, std::slice::from_ref(value))
                .unwrap()
                .take()
                .unwrap()
        };
        assert!(is_some(&some) && !is_some(&none));
        let page: String = eval(&BuiltinOp::Unwrap, &[some]).unwrap().take().unwrap();
        assert_eq!(page, "page");
        assert!(eval(&BuiltinOp::Unwrap, &[none]).is_err());
    }
}
