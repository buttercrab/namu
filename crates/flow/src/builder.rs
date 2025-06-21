//! The `Builder` API for constructing a computational graph.
//!
//! This module is used by the `#[workflow]` procedural macro to translate
//! Rust code into a graph's Intermediate Representation (IR).

use std::any::{TypeId, type_name};
use std::cell::{Ref, RefCell, RefMut};
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::Arc;

use namu_core::ValueId;

use crate::graph::{Graph, NodeArena, TracedValue, ValueArena};
use crate::ir::{BasicBlock, BlockId, NodeKind, Terminator};

// --- Builder API ---

struct BuilderInner {
    node_arena: NodeArena,
    val_arena: ValueArena,
    blocks: Vec<BasicBlock>,
    current_block_id: BlockId,
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
            }),
            _phantom: PhantomData,
        }
    }

    pub fn current_block_id(&self) -> BlockId {
        self.inner.borrow().current_block_id
    }

    pub fn arena(&self) -> Ref<NodeArena> {
        Ref::map(self.inner.borrow(), |inner| &inner.node_arena)
    }

    pub fn arena_mut(&self) -> RefMut<NodeArena> {
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

    pub fn literal<L: Debug + Send + Sync + 'static>(&self, value: L) -> ValueId {
        let debug_repr = format!("{:?}", value);
        let kind = NodeKind::literal(Arc::new(value), debug_repr);
        self.add_node(kind, 1)[0]
    }

    pub fn phi(&self, from: Vec<(BlockId, ValueId)>) -> ValueId {
        let kind = NodeKind::phi(from);
        self.add_node(kind, 1)[0]
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
        Graph::new(inner.node_arena, inner.blocks)
    }
}

pub fn call<Env: 'static, T: 'static>(
    builder: &Builder<Env>,
    task_id: &str,
    _debug_loc: String,
    inputs: Vec<ValueId>,
) -> TracedValue<T> {
    let outs = builder.call(task_id.to_string(), inputs, 1);
    TracedValue::new(outs[0])
}

macro_rules! define_call {
    ($fname:ident, $arity:expr, [$($idx:tt),*], [$($T:ident),*]) => {
        #[allow(non_snake_case)]
        #[allow(unused_variables)]
        pub fn $fname<Env: 'static, $( $T: 'static ),*>(
            builder: &Builder<Env>,
            task_id: &str,
            _debug_loc: String,
            inputs: Vec<ValueId>,
        ) -> ( $( TracedValue<$T>, )* ) {
            let outs = builder.call(task_id.to_string(), inputs, $arity);
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

pub fn literal<T: Debug + Send + Sync + 'static, U>(
    builder: &Builder<U>,
    value: T,
) -> TracedValue<T> {
    let id = builder.literal(value);
    TracedValue::new(id)
}

pub fn phi<G: 'static, T: Clone + 'static>(
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
