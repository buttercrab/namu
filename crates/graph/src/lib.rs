use std::any::Any;
use std::collections::HashMap;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::Arc;

pub use macros::{trace, workflow};

pub type Value = Arc<dyn Any + Send + Sync>;
pub type Executable = Arc<dyn Fn(Vec<Value>) -> Value + Send + Sync>;

pub enum Node {
    Call {
        name: &'static str,
        func: Executable,
        parents: Vec<Arc<Node>>,
    },
    Literal {
        value: Value,
        debug_repr: String,
    },
    If {
        cond: Arc<Node>,
        then_branch: Arc<Node>,
        else_branch: Arc<Node>,
    },
}

#[derive(Clone)]
pub struct TraceValue<T> {
    pub node: Arc<Node>,
    _phantom: PhantomData<T>,
}

impl<T> TraceValue<T> {
    pub fn graph_string(&self) -> String {
        let mut assignments = HashMap::<*const Node, String>::new();
        let mut lines = Vec::<String>::new();
        let mut counter = 0;
        let final_var =
            self.to_pseudocode_recursive(&self.node, &mut assignments, &mut lines, &mut counter);

        lines.push(format!("\n// Result: {}", final_var));
        lines.join("\n")
    }

    fn to_pseudocode_recursive(
        &self,
        node_arc: &Arc<Node>,
        assignments: &mut HashMap<*const Node, String>,
        lines: &mut Vec<String>,
        counter: &mut usize,
    ) -> String {
        let node_ptr = Arc::as_ptr(node_arc);
        if let Some(var_name) = assignments.get(&node_ptr) {
            return var_name.clone();
        }

        let current_var_name = format!("var{}", *counter);
        *counter += 1;

        assignments.insert(node_ptr, current_var_name.clone());

        let line = match &**node_arc {
            Node::Literal { debug_repr, .. } => {
                format!("let {} = {};", current_var_name, debug_repr)
            }
            Node::Call { name, parents, .. } => {
                let parent_vars: Vec<String> = parents
                    .iter()
                    .map(|p| self.to_pseudocode_recursive(p, assignments, lines, counter))
                    .collect();
                format!(
                    "let {} = {}({});",
                    current_var_name,
                    name,
                    parent_vars.join(", ")
                )
            }
            Node::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let cond_var = self.to_pseudocode_recursive(cond, assignments, lines, counter);
                let then_var =
                    self.to_pseudocode_recursive(then_branch, assignments, lines, counter);
                let else_var =
                    self.to_pseudocode_recursive(else_branch, assignments, lines, counter);
                format!(
                    "let {} = if {} then {} else {};",
                    current_var_name, cond_var, then_var, else_var
                )
            }
        };

        lines.push(line);
        current_var_name
    }

    pub fn run(&self) -> T
    where
        T: Clone + 'static,
    {
        let mut results = HashMap::<*const Node, Value>::new();
        let final_value = self.run_node(&self.node, &mut results);
        final_value.downcast_ref::<T>().unwrap().clone()
    }

    fn run_node(&self, node_arc: &Arc<Node>, results: &mut HashMap<*const Node, Value>) -> Value {
        let node_ptr = Arc::as_ptr(node_arc);
        if let Some(value) = results.get(&node_ptr) {
            return value.clone();
        }

        let value = match &**node_arc {
            Node::Literal { value, .. } => value.clone(),
            Node::Call { func, parents, .. } => {
                let inputs = parents
                    .iter()
                    .map(|p| self.run_node(p, results))
                    .collect::<Vec<_>>();
                func(inputs)
            }
            Node::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let cond_result = self.run_node(cond, results);
                if *cond_result.downcast_ref::<bool>().unwrap() {
                    self.run_node(then_branch, results)
                } else {
                    self.run_node(else_branch, results)
                }
            }
        };

        results.insert(node_ptr, value.clone());
        value
    }
}

pub fn new_call<T>(name: &'static str, func: Executable, parents: Vec<Arc<Node>>) -> TraceValue<T> {
    TraceValue {
        node: Arc::new(Node::Call {
            name,
            func,
            parents,
        }),
        _phantom: PhantomData,
    }
}

pub fn new_literal<T: Debug + Send + Sync + 'static>(value: T) -> TraceValue<T> {
    let debug_repr = format!("{:?}", value);
    TraceValue {
        node: Arc::new(Node::Literal {
            value: Arc::new(value),
            debug_repr,
        }),
        _phantom: PhantomData,
    }
}

pub fn graph_if<T: Clone + Send + Sync + Debug + 'static>(
    cond: TraceValue<bool>,
    then_branch: TraceValue<T>,
    else_branch: TraceValue<T>,
) -> TraceValue<T> {
    TraceValue {
        node: Arc::new(Node::If {
            cond: cond.node,
            then_branch: then_branch.node,
            else_branch: else_branch.node,
        }),
        _phantom: PhantomData,
    }
}
