//! The `Builder` API for constructing a computational graph.
//!
//! This module is used by the `#[workflow]` procedural macro to translate
//! Rust code into a graph's Intermediate Representation (IR).

use std::cell::{Ref, RefCell, RefMut};
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::Arc;

use crate::ir::{BasicBlock, BlockId, Graph, NodeArena, NodeId, NodeKind, Terminator, TracedValue};

// --- Builder API ---

struct BuilderInner {
    arena: NodeArena,
    blocks: Vec<BasicBlock>,
    current_block_id: BlockId,
}

pub fn literal<T: Debug + Send + Sync + 'static, U>(
    builder: &Builder<U>,
    value: T,
) -> TracedValue<T> {
    let debug_repr = format!("{:?}", value);
    let kind = NodeKind::Literal {
        value: Arc::new(value),
        debug_repr,
    };
    builder.add_instruction(kind)
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

    builder.add_instruction(NodeKind::Phi { from })
}

pub fn extract<G: 'static, T: 'static>(
    builder: &Builder<G>,
    tuple: TracedValue<()>,
    index: usize,
) -> TracedValue<T> {
    builder.add_instruction(NodeKind::Extract {
        tuple: tuple.id,
        index,
    })
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
                arena: NodeArena::default(),
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
        Ref::map(self.inner.borrow(), |inner| &inner.arena)
    }

    pub fn arena_mut(&self) -> RefMut<NodeArena> {
        RefMut::map(self.inner.borrow_mut(), |inner| &mut inner.arena)
    }

    pub fn add_instruction<V>(&self, kind: NodeKind) -> TracedValue<V> {
        let mut inner = self.inner.borrow_mut();
        let id = inner.arena.new_node(kind);
        let current_block_id = inner.current_block_id;
        inner.blocks[current_block_id].instructions.push(id);
        TracedValue::new(id)
    }

    pub fn add_instruction_to_current_block(&self, node_id: NodeId) {
        let mut inner = self.inner.borrow_mut();
        let current_block_id = inner.current_block_id;
        inner.blocks[current_block_id].instructions.push(node_id);
    }

    pub fn seal_block(&self, terminator: Terminator) {
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
        Graph::new(inner.arena, inner.blocks)
    }
}
