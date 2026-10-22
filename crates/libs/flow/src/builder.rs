//! The `Builder` API for constructing a computational graph.
//!
//! This module is used by the `#[workflow]` procedural macro to translate
//! Rust code into a graph's Intermediate Representation (IR).

use std::any::{TypeId, type_name};
use std::cell::{Ref, RefCell, RefMut};
use std::fmt::Debug;
use std::marker::PhantomData;

use namu_core::ValueId;
use namu_core::ir::{BuiltinOp, ScalarType};
use namu_core::types::{Defs, Type};
use namu_core::value::Data;
use serde::Serialize;

use crate::graph::{Graph, NodeArena, TracedValue, ValueArena};
use crate::ir::{BasicBlock, BlockId, NodeKind, Terminator};

// --- Builder API ---

struct BuilderInner {
    node_arena: NodeArena,
    val_arena: ValueArena,
    blocks: Vec<BasicBlock>,
    current_block_id: BlockId,
    /// Each value's type, by value id, as it is made.
    types: Vec<Option<Type>>,
    defs: Defs,
}
pub struct Builder<T> {
    inner: RefCell<BuilderInner>,
    _phantom: PhantomData<T>,
}

impl<T> Default for Builder<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Builder<T> {
    pub fn new() -> Self {
        Self {
            inner: RefCell::new(BuilderInner {
                node_arena: NodeArena::default(),
                val_arena: ValueArena::default(),
                blocks: vec![BasicBlock::default()],
                current_block_id: 0,
                types: Vec::new(),
                defs: Defs::new(),
            }),
            _phantom: PhantomData,
        }
    }

    /// Records that `value` is a `V`.
    pub fn typed<V: Data>(&self, value: ValueId) {
        let mut inner = self.inner.borrow_mut();
        let inner = &mut *inner;
        let ty = V::describe(&mut inner.defs);
        if inner.types.len() <= value {
            inner.types.resize(value + 1, None);
        }
        inner.types[value] = Some(ty);
    }

    pub fn current_block_id(&self) -> BlockId {
        self.inner.borrow().current_block_id
    }

    pub fn arena(&self) -> Ref<'_, NodeArena> {
        Ref::map(self.inner.borrow(), |inner| &inner.node_arena)
    }

    pub fn arena_mut(&self) -> RefMut<'_, NodeArena> {
        RefMut::map(self.inner.borrow_mut(), |inner| &mut inner.node_arena)
    }

    fn add_node(&self, kind: NodeKind, arity: usize) -> Vec<ValueId> {
        let mut inner = self.inner.borrow_mut();
        let outputs = inner.val_arena.new_values(arity);
        let node_id = inner.node_arena.new_node(kind, outputs.clone());
        let current_block_id = inner.current_block_id;
        inner.blocks[current_block_id].instructions.push(node_id);
        outputs
    }

    pub fn call(&self, task_id: String, inputs: Vec<ValueId>, arity: usize) -> Vec<ValueId> {
        let kind = NodeKind::call(task_id, inputs);
        self.add_node(kind, arity)
    }

    pub fn literal<L: Serialize + Debug + Send + Sync + 'static>(&self, value: L) -> ValueId {
        // Literals are stored as JSON so every runtime decodes them the same way.
        let json = serde_json::to_string(&value)
            .unwrap_or_else(|err| panic!("literal {value:?} is not representable as JSON: {err}"));
        self.add_node(NodeKind::literal(json), 1)[0]
    }

    /// Emits a fan-out over the list `input` and returns the per-element value.
    pub fn fan_out(&self, input: ValueId) -> ValueId {
        self.add_node(NodeKind::FanOut { input }, 1)[0]
    }

    /// Emits a fan-in of `input` over the innermost fan-out scope and returns the list value.
    pub fn collect(&self, input: ValueId) -> ValueId {
        self.add_node(
            NodeKind::Collect {
                input,
                reduce: None,
            },
            1,
        )[0]
    }

    /// Emits a window of `size` items of `input` and returns the per-window list.
    pub fn window(&self, input: ValueId, size: u64) -> ValueId {
        self.add_node(NodeKind::Window { input, size }, 1)[0]
    }

    /// Emits a fan-in that folds `input` from every branch with `task_id`, from `init`, and
    /// returns the last accumulator.
    pub fn reduce(&self, task_id: String, init: ValueId, input: ValueId) -> ValueId {
        let reduce = Some((task_id, init));
        self.add_node(NodeKind::Collect { input, reduce }, 1)[0]
    }

    /// Emits a built-in expression over `inputs`, read as `ty`, and returns its value.
    pub fn builtin(&self, op: BuiltinOp, ty: ScalarType, inputs: Vec<ValueId>) -> ValueId {
        self.add_node(NodeKind::Builtin { op, ty, inputs }, 1)[0]
    }

    pub fn phi(&self, from: Vec<(BlockId, ValueId)>) -> ValueId {
        let kind = NodeKind::phi(from);
        self.add_node(kind, 1)[0]
    }

    /// Gives the call that ends the current block an error edge, and seals the block: control
    /// goes to `ok` if the task succeeds, else to `err`. Returns the error message's value.
    pub fn catch(&self, ok: BlockId, err: BlockId) -> ValueId {
        let error = {
            let mut inner = self.inner.borrow_mut();
            let inner = &mut *inner;
            let last = inner.blocks[inner.current_block_id].instructions.last();
            assert!(
                last.is_some_and(|&node| {
                    matches!(inner.node_arena.nodes[node].kind, NodeKind::Call { .. })
                }),
                "only a task call can be attempted"
            );
            inner.val_arena.new_value()
        };
        self.seal_block(Terminator::Catch { ok, err, error });
        error
    }

    fn seal_block(&self, terminator: Terminator) {
        let mut inner = self.inner.borrow_mut();
        let current_block_id = inner.current_block_id;
        let current_block = &mut inner.blocks[current_block_id];
        assert!(current_block.terminator.is_none(), "Block already sealed");
        current_block.terminator = Some(terminator);
    }

    pub fn new_block(&self) -> BlockId {
        let mut inner = self.inner.borrow_mut();
        let id = inner.blocks.len();
        inner.blocks.push(BasicBlock::default());
        id
    }

    pub fn switch_to_block(&self, block_id: BlockId) {
        let mut inner = self.inner.borrow_mut();
        inner.current_block_id = block_id;
    }

    pub fn build(self) -> Graph<T> {
        let inner = self.inner.into_inner();
        Graph::new(inner.node_arena, inner.blocks, inner.types, inner.defs)
    }
}

pub fn call<Env: 'static, T: Data>(
    builder: &Builder<Env>,
    task_id: &str,
    inputs: Vec<ValueId>,
) -> TracedValue<T> {
    let outs = builder.call(task_id.to_string(), inputs, 1);
    builder.typed::<T>(outs[0]);
    TracedValue::new(outs[0])
}

macro_rules! define_call {
    ($fname:ident, $arity:expr, [$($idx:tt),*], [$($T:ident),*]) => {
        #[allow(non_snake_case)]
        #[allow(unused_variables)]
        pub fn $fname<Env: 'static, $( $T: Data ),*>(
            builder: &Builder<Env>,
            task_id: &str,
            inputs: Vec<ValueId>,
        ) -> ( $( TracedValue<$T>, )* ) {
            let outs = builder.call(task_id.to_string(), inputs, $arity);
            $( builder.typed::<$T>(outs[$idx]); )*
            #[allow(clippy::unused_unit)]
            ( $( TracedValue::new(outs[$idx]), )*)
        }
    };
}

// Generate helpers up to arity 9
define_call!(call0, 0, [], []);
define_call!(call1, 1, [0], [A]);
define_call!(call2, 2, [0, 1], [A, B]);
define_call!(call3, 3, [0, 1, 2], [A, B, C]);
define_call!(call4, 4, [0, 1, 2, 3], [A, B, C, D]);
define_call!(call5, 5, [0, 1, 2, 3, 4], [A, B, C, D, E]);
define_call!(call6, 6, [0, 1, 2, 3, 4, 5], [A, B, C, D, E, F]);
define_call!(call7, 7, [0, 1, 2, 3, 4, 5, 6], [A, B, C, D, E, F, G]);
define_call!(call8, 8, [0, 1, 2, 3, 4, 5, 6, 7], [A, B, C, D, E, F, G, H]);
define_call!(
    call9,
    9,
    [0, 1, 2, 3, 4, 5, 6, 7, 8],
    [A, B, C, D, E, F, G, H, I]
);

/// A task argument. The impl is chosen by rustc from the parameter type, which is what makes
/// fan-out and fan-in type-driven:
///
/// - `TracedValue<T>` passed as `T`: a plain argument.
/// - `TracedValue<Vec<T>>` passed as `T`: fan-out, one branch per element.
/// - `TracedValue<T>` passed as `Vec<T>`: fan-in, collecting every branch of the current scope.
pub trait IntoArg<T> {
    fn into_arg<G>(self, builder: &Builder<G>) -> ValueId;
}

impl<T> IntoArg<T> for TracedValue<T> {
    fn into_arg<G>(self, _builder: &Builder<G>) -> ValueId {
        self.id
    }
}

impl<T: Data> IntoArg<T> for TracedValue<Vec<T>> {
    fn into_arg<G>(self, builder: &Builder<G>) -> ValueId {
        let item = builder.fan_out(self.id);
        builder.typed::<T>(item);
        item
    }
}

impl<T: Data> IntoArg<Vec<T>> for TracedValue<T> {
    fn into_arg<G>(self, builder: &Builder<G>) -> ValueId {
        let list = builder.collect(self.id);
        builder.typed::<Vec<T>>(list);
        list
    }
}

/// `window(item, size)`: continues once per `size` items of the innermost fan-out, with them, in
/// branch order.
pub fn window<G, T: Data>(
    builder: &Builder<G>,
    item: TracedValue<T>,
    size: u64,
) -> TracedValue<Vec<T>> {
    assert!(size > 0, "a window holds at least one item");
    let output = builder.window(item.id, size);
    builder.typed::<Vec<T>>(output);
    TracedValue::new(output)
}

/// `each(list)`: one branch per item of `list` (as it arrives, for a stream's), each with its
/// item. What passing a list to a task taking one item does, without the task.
pub fn each<G, T: Data>(builder: &Builder<G>, list: TracedValue<Vec<T>>) -> TracedValue<T> {
    TracedValue::new(IntoArg::<T>::into_arg(list, builder))
}

/// A reduce task's call: folds `item` over every branch of the innermost fan-out, from `init`.
pub fn reduce<G, A: Data, T>(
    builder: &Builder<G>,
    task_id: &str,
    init: impl IntoArg<A>,
    item: TracedValue<T>,
) -> TracedValue<A> {
    let init = init.into_arg(builder);
    let output = builder.reduce(task_id.to_string(), init, item.id);
    builder.typed::<A>(output);
    TracedValue::new(output)
}

pub fn literal<T: Data + Debug, U>(builder: &Builder<U>, value: T) -> TracedValue<T> {
    let id = builder.literal(value);
    builder.typed::<T>(id);
    TracedValue::new(id)
}

/// `value`, made by hand, typed as `like` is.
pub fn typed_like<G, T: Data>(
    builder: &Builder<G>,
    value: ValueId,
    _like: &TracedValue<T>,
) -> TracedValue<T> {
    builder.typed::<T>(value);
    TracedValue::new(value)
}

pub fn phi<G: 'static, T: Data>(
    builder: &Builder<G>,
    from: Vec<(BlockId, TracedValue<T>)>,
) -> TracedValue<T> {
    if from.is_empty() {
        panic!("phi node must have at least one incoming value");
    }

    let first_node_id = from[0].1.id;
    if from
        .iter()
        .skip(1)
        .all(|(_, value)| value.id == first_node_id)
    {
        return TracedValue::new(first_node_id);
    }

    let from = from
        .into_iter()
        .map(|(block, value)| (block, value.id))
        .collect();

    let id = builder.phi(from);
    builder.typed::<T>(id);
    TracedValue::new(id)
}

pub fn jump<T>(builder: &Builder<T>, target: BlockId) {
    builder.seal_block(Terminator::jump(target));
}

pub fn branch<T>(
    builder: &Builder<T>,
    condition: TracedValue<bool>,
    true_target: BlockId,
    false_target: BlockId,
) {
    builder.seal_block(Terminator::branch(condition.id, true_target, false_target));
}

/// Ends the current block with the call just emitted, which may fail: see [`Builder::catch`].
pub fn catch<T>(builder: &Builder<T>, ok: BlockId, err: BlockId) -> TracedValue<String> {
    let error = builder.catch(ok, err);
    builder.typed::<String>(error);
    TracedValue::new(error)
}

pub fn return_value<T: 'static, U: 'static>(builder: &Builder<T>, value: TracedValue<U>) {
    debug_assert_eq!(
        TypeId::of::<T>(),
        TypeId::of::<U>(),
        "return_value: {} and {} must be the same type",
        type_name::<T>(),
        type_name::<U>(),
    );
    builder.seal_block(Terminator::return_value(value.id));
}

pub fn return_unit<T>(builder: &Builder<T>) {
    builder.seal_block(Terminator::return_unit());
}

// --- Built-in expressions ---

/// A type built-in expressions work on.
pub trait Scalar: Data {
    const TYPE: ScalarType;
}

macro_rules! scalar {
    ($($t:ty => $variant:ident),*) => {$(
        impl Scalar for $t {
            const TYPE: ScalarType = ScalarType::$variant;
        }
    )*};
}

scalar!(bool => Bool, i32 => I32, i64 => I64, u32 => U32, u64 => U64, f32 => F32, f64 => F64,
    String => String);

/// Numbers: what arithmetic is defined on.
pub trait Number: Scalar {}

impl Number for i32 {}
impl Number for i64 {}
impl Number for u32 {}
impl Number for u64 {}
impl Number for f32 {}
impl Number for f64 {}

/// What `+` is defined on: numbers, and strings (concatenation).
pub trait Plus: Scalar {}

impl<T: Number> Plus for T {}
impl Plus for String {}

/// Numbers that have a negative: what unary `-` is defined on.
pub trait Signed: Number {}

impl Signed for i32 {}
impl Signed for i64 {}
impl Signed for f32 {}
impl Signed for f64 {}

fn builtin<G, T: Scalar, R: Data>(
    builder: &Builder<G>,
    op: BuiltinOp,
    inputs: &[ValueId],
) -> TracedValue<R> {
    let id = builder.builtin(op, T::TYPE, inputs.to_vec());
    builder.typed::<R>(id);
    TracedValue::new(id)
}

/// `a + b`.
pub fn plus<G, T: Plus>(b: &Builder<G>, x: TracedValue<T>, y: TracedValue<T>) -> TracedValue<T> {
    builtin::<G, T, T>(b, BuiltinOp::Add, &[x.id, y.id])
}

/// `a - b`, `a * b`, `a / b`, `a % b`.
pub fn arith<G, T: Number>(
    b: &Builder<G>,
    op: BuiltinOp,
    x: TracedValue<T>,
    y: TracedValue<T>,
) -> TracedValue<T> {
    builtin::<G, T, T>(b, op, &[x.id, y.id])
}

/// `-a`.
pub fn negate<G, T: Signed>(b: &Builder<G>, x: TracedValue<T>) -> TracedValue<T> {
    builtin::<G, T, T>(b, BuiltinOp::Neg, &[x.id])
}

/// `a == b`, `a != b`, `a < b`, `a <= b`, `a > b`, `a >= b`.
pub fn compare<G, T: Scalar>(
    b: &Builder<G>,
    op: BuiltinOp,
    x: TracedValue<T>,
    y: TracedValue<T>,
) -> TracedValue<bool> {
    builtin::<G, T, bool>(b, op, &[x.id, y.id])
}

/// `a && b`, `a || b`. Both sides are evaluated: they are values, which have no effects.
pub fn logic<G>(
    b: &Builder<G>,
    op: BuiltinOp,
    x: TracedValue<bool>,
    y: TracedValue<bool>,
) -> TracedValue<bool> {
    builtin::<G, bool, bool>(b, op, &[x.id, y.id])
}

/// `!a`.
pub fn not<G>(b: &Builder<G>, x: TracedValue<bool>) -> TracedValue<bool> {
    builtin::<G, bool, bool>(b, BuiltinOp::Not, &[x.id])
}
