//! What built-in expressions mean (design §4.3): the engine evaluates them where a branch stands,
//! with no task call. Operands are read as the expression's [`ScalarType`] and the result is a
//! value of that type (or a `bool`, for comparisons and logic). Integer arithmetic is checked:
//! an overflow, or a division by zero, is an error, which fails the branch, rather than a wrong
//! number.

use anyhow::{Context, anyhow, bail};

use crate::ir::{BuiltinOp, ScalarType};
use crate::value::{Data, Value};

/// Evaluates `op` on `inputs`, read as `ty`.
pub fn eval(op: BuiltinOp, ty: ScalarType, inputs: &[Value]) -> anyhow::Result<Value> {
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

fn read<T: Data, const N: usize>(op: BuiltinOp, inputs: &[Value]) -> anyhow::Result<[T; N]> {
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
fn compare<T: Data + PartialOrd>(op: BuiltinOp, inputs: &[Value]) -> anyhow::Result<Option<Value>> {
    let ordering = |test: fn(&T, &T) -> bool| -> anyhow::Result<Option<Value>> {
        let [a, b] = read::<T, 2>(op, inputs)?;
        Ok(Some(Value::new(test(&a, &b))))
    };
    match op {
        BuiltinOp::Eq => ordering(T::eq),
        BuiltinOp::Ne => ordering(T::ne),
        BuiltinOp::Lt => ordering(T::lt),
        BuiltinOp::Le => ordering(T::le),
        BuiltinOp::Gt => ordering(T::gt),
        BuiltinOp::Ge => ordering(T::ge),
        _ => Ok(None),
    }
}

type Checked<T> = fn(T, T) -> Option<T>;

#[allow(clippy::too_many_arguments)]
fn integer<T: Data + PartialOrd + Copy>(
    op: BuiltinOp,
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
        BuiltinOp::Add => checked(add, "overflow adding"),
        BuiltinOp::Sub => checked(sub, "overflow subtracting"),
        BuiltinOp::Mul => checked(mul, "overflow multiplying"),
        BuiltinOp::Div => checked(div, "division by zero or overflow dividing"),
        BuiltinOp::Rem => checked(rem, "division by zero or overflow in a remainder of"),
        BuiltinOp::Neg => {
            let [a] = read::<T, 1>(op, inputs)?;
            neg(a)
                .map(Value::new)
                .ok_or_else(|| anyhow!("overflow negating {}", std::any::type_name::<T>()))
        }
        _ => bail!("{op:?} is not defined on {}", std::any::type_name::<T>()),
    }
}

fn float<T>(op: BuiltinOp, inputs: &[Value]) -> anyhow::Result<Value>
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
        BuiltinOp::Add => arith(T::add),
        BuiltinOp::Sub => arith(T::sub),
        BuiltinOp::Mul => arith(T::mul),
        BuiltinOp::Div => arith(T::div),
        BuiltinOp::Rem => arith(T::rem),
        BuiltinOp::Neg => {
            let [a] = read::<T, 1>(op, inputs)?;
            Ok(Value::new(-a))
        }
        _ => bail!("{op:?} is not defined on {}", std::any::type_name::<T>()),
    }
}

fn logic(op: BuiltinOp, inputs: &[Value]) -> anyhow::Result<Value> {
    if let Some(value) = compare::<bool>(op, inputs)? {
        return Ok(value);
    }
    match op {
        BuiltinOp::And => read::<bool, 2>(op, inputs).map(|[a, b]| Value::new(a && b)),
        BuiltinOp::Or => read::<bool, 2>(op, inputs).map(|[a, b]| Value::new(a || b)),
        BuiltinOp::Not => read::<bool, 1>(op, inputs).map(|[a]| Value::new(!a)),
        _ => bail!("{op:?} is not defined on bool"),
    }
}

fn string(op: BuiltinOp, inputs: &[Value]) -> anyhow::Result<Value> {
    if let Some(value) = compare::<String>(op, inputs)? {
        return Ok(value);
    }
    match op {
        BuiltinOp::Add => read::<String, 2>(op, inputs).map(|[a, b]| Value::new(a + &b)),
        _ => bail!("{op:?} is not defined on String"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run<T: Data>(op: BuiltinOp, ty: ScalarType, inputs: &[Value]) -> anyhow::Result<T> {
        eval(op, ty, inputs)?.take()
    }

    #[test]
    fn integers_are_checked() {
        let (two, three) = (Value::new(2i64), Value::new(3i64));
        assert_eq!(
            run::<i64>(
                BuiltinOp::Add,
                ScalarType::I64,
                &[two.clone(), three.clone()]
            )
            .unwrap(),
            5
        );
        assert_eq!(
            run::<i64>(
                BuiltinOp::Rem,
                ScalarType::I64,
                &[three.clone(), two.clone()]
            )
            .unwrap(),
            1
        );
        assert!(run::<bool>(BuiltinOp::Lt, ScalarType::I64, &[two.clone(), three]).unwrap());
        let max = Value::new(i32::MAX);
        let overflow = run::<i32>(BuiltinOp::Add, ScalarType::I32, &[max, Value::new(1i32)]);
        assert!(overflow.unwrap_err().to_string().contains("overflow"));
        let zero = run::<i64>(BuiltinOp::Div, ScalarType::I64, &[two, Value::new(0i64)]);
        assert!(zero.unwrap_err().to_string().contains("division by zero"));
        assert!(
            run::<u32>(
                BuiltinOp::Sub,
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
                BuiltinOp::Mul,
                ScalarType::I64,
                &[ten.clone(), Value::new(3i64)]
            )
            .unwrap(),
            30
        );
        assert_eq!(
            run::<f64>(BuiltinOp::Div, ScalarType::F64, &[ten, Value::new(4.0f64)]).unwrap(),
            2.5
        );
    }

    #[test]
    fn logic_and_strings() {
        let (t, f) = (Value::new(true), Value::new(false));
        assert!(!run::<bool>(BuiltinOp::And, ScalarType::Bool, &[t.clone(), f.clone()]).unwrap());
        assert!(run::<bool>(BuiltinOp::Or, ScalarType::Bool, &[t.clone(), f]).unwrap());
        assert!(!run::<bool>(BuiltinOp::Not, ScalarType::Bool, &[t]).unwrap());
        let (a, b) = (Value::new("na".to_string()), Value::new("mu".to_string()));
        assert_eq!(
            run::<String>(BuiltinOp::Add, ScalarType::String, &[a.clone(), b.clone()]).unwrap(),
            "namu"
        );
        assert!(run::<bool>(BuiltinOp::Gt, ScalarType::String, &[a.clone(), b.clone()]).unwrap());
        assert!(eval(BuiltinOp::Sub, ScalarType::String, &[a, b]).is_err());
    }
}
