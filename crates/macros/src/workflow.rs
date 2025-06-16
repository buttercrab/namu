use std::collections::BTreeSet;

use proc_macro::TokenStream;
use proc_macro_error::abort;
use quote::{format_ident, quote};
use syn::{
    Block, Expr, ExprIf, Ident, ItemFn, Pat, ReturnType, Stmt, parse_macro_input, parse_quote,
    visit_mut::{self, VisitMut},
};

struct WorkflowVisitor {
    // We use BTreeSet for deterministic ordering of variables.
    scopes: Vec<BTreeSet<Ident>>,
    next_control_flow_id: usize,
    builder_ident: Ident,
    last_expr_has_value: bool,
}

impl WorkflowVisitor {
    fn new(builder_ident: Ident) -> Self {
        Self {
            scopes: vec![],
            next_control_flow_id: 0,
            builder_ident,
            last_expr_has_value: false,
        }
    }

    fn new_control_flow_id(&mut self) -> usize {
        let id = self.next_control_flow_id;
        self.next_control_flow_id += 1;
        id
    }

    fn enter_scope(&mut self) {
        self.scopes.push(Default::default());
    }

    fn exit_scope(&mut self) {
        self.scopes.pop();
    }

    fn insert_var(&mut self, name: Ident) {
        self.scopes.last_mut().unwrap().insert(name);
    }

    fn list_vars(&self) -> Vec<Ident> {
        self.scopes
            .iter()
            .map(|x| x.iter().cloned())
            .flatten()
            .collect()
    }
}

impl VisitMut for WorkflowVisitor {
    fn visit_block_mut(&mut self, i: &mut Block) {
        self.enter_scope();
        for stmt in i.stmts.iter_mut() {
            self.visit_stmt_mut(stmt);
        }
        self.exit_scope();

        self.last_expr_has_value = self.last_expr_has_value
            && i.stmts
                .last()
                .is_some_and(|stmt| matches!(stmt, Stmt::Expr(_, None)));
    }

    fn visit_expr_mut(&mut self, i: &mut Expr) {
        match i {
            Expr::Call(call_expr) => {
                for arg in call_expr.args.iter_mut() {
                    self.visit_expr_mut(arg);
                }

                let builder_ident = &self.builder_ident;
                call_expr.args.insert(0, parse_quote! { &#builder_ident });

                self.last_expr_has_value = true;
            }
            Expr::Path(_) => {
                self.last_expr_has_value = true;
            }
            Expr::Assign(expr) => {
                abort!(
                    expr,
                    "assignments are not supported in expressions, put semicolon after the assignment"
                );
            }
            Expr::If(if_expr) => {
                *i = self.handle_if(if_expr);
            }
            Expr::While(while_expr) => {
                *i = self.handle_while(while_expr);
                self.last_expr_has_value = false;
            }
            Expr::Lit(lit) => {
                let builder_ident = &self.builder_ident;
                *i = parse_quote! { graph::new_literal(&#builder_ident, #lit) };
                self.last_expr_has_value = true;
            }
            _ => visit_mut::visit_expr_mut(self, i),
        }
    }

    fn visit_stmt_mut(&mut self, i: &mut Stmt) {
        match i {
            Stmt::Local(local) => {
                let Pat::Ident(pat_ident) = &local.pat else {
                    abort!(local, "only simple idents are supported in let bindings");
                };
                if let Some(init) = &mut local.init {
                    let is_mut = pat_ident.mutability.is_some();
                    self.visit_expr_mut(&mut init.expr);
                    if is_mut {
                        self.insert_var(pat_ident.ident.clone());
                    }
                } else {
                    abort!(local, "let bindings must be initialized");
                }

                self.last_expr_has_value = false;
            }
            Stmt::Expr(Expr::Assign(assign_expr), _semi) => {
                let Expr::Path(path) = &*assign_expr.left else {
                    abort!(
                        assign_expr,
                        "only simple idents are supported in let bindings"
                    );
                };
                let Some(name) = path.path.get_ident() else {
                    abort!(path, "only simple idents are supported in let bindings");
                };

                self.visit_expr_mut(&mut assign_expr.right);

                let right = &assign_expr.right;
                let new_assign: Stmt = parse_quote! {
                    #name = #right;
                };
                *i = new_assign;

                self.last_expr_has_value = false;
            }
            Stmt::Expr(expr, _semi) => self.visit_expr_mut(expr),
            _ => visit_mut::visit_stmt_mut(self, i),
        }
    }
}

impl WorkflowVisitor {
    fn handle_if(&mut self, if_expr: &mut ExprIf) -> Expr {
        let builder = self.builder_ident.clone();
        let id = self.new_control_flow_id();

        let vars = self.list_vars();

        self.visit_expr_mut(&mut if_expr.cond);
        let cond = &if_expr.cond;

        let pre_if_captures = vars.iter().map(|name| {
            let pre_if_name = format_ident!("__pre_if_{}_{}", name, id);
            quote! { let #pre_if_name = #name; }
        });

        let then_block_id = format_ident!("__if_then_block_{}", id);
        let merge_block_id = format_ident!("__if_merge_block_{}", id);
        let parent_predecessor_id = format_ident!("__if_parent_predecessor_{}", id);

        self.visit_block_mut(&mut if_expr.then_branch);
        let then_branch_body = &if_expr.then_branch;
        let then_has_value = self.last_expr_has_value;

        let then_post_captures = vars.iter().map(|name| {
            let post_then_name = format_ident!("__post_then_{}_{}", name, id);
            quote! { let #post_then_name = #name; }
        });

        let (else_block_setup, else_block_impl, false_target, phi) =
            if let Some((_, else_expr)) = &mut if_expr.else_branch {
                let else_block_id = format_ident!("__if_else_block_{}", id);

                let else_resets = vars.iter().map(|name| {
                    let pre_if_name = format_ident!("__pre_if_{}_{}", name, id);
                    quote! { #name = #pre_if_name; }
                });

                self.visit_expr_mut(else_expr);
                let else_has_value = self.last_expr_has_value;

                let else_post_captures = vars.iter().map(|name| {
                    let post_else_name = format_ident!("__post_else_{}_{}", name, id);
                    quote! { let #post_else_name = #name; }
                });

                let setup = quote! { let #else_block_id = #builder.new_block(); };
                let else_predecessor_id = format_ident!("__else_predecessor_id_{}", id);
                let implementation = quote! {
                    #builder.switch_to_block(#else_block_id);
                    let __else_val = {
                        #(#else_resets)*
                        #else_expr
                    };
                    #(#else_post_captures)*
                    let #else_predecessor_id = #builder.current_block_id();
                    #builder.seal_block(graph::Terminator::jump(#merge_block_id));
                };

                let then_predecessor_id = format_ident!("__then_predecessor_id_{}", id);
                let phi = if then_has_value && else_has_value {
                    Some(quote! {
                         graph::phi(
                            &#builder,
                            vec![
                                (#then_predecessor_id, __then_val),
                                (#else_predecessor_id, __else_val)
                            ]
                        )
                    })
                } else {
                    None
                };

                (Some(setup), Some(implementation), else_block_id, phi)
            } else {
                (None, None, merge_block_id.clone(), None)
            };

        let merge_phis = vars.iter().map(|name| {
            let pre_if_name = format_ident!("__pre_if_{}_{}", name, id);
            let post_then_name = format_ident!("__post_then_{}_{}", name, id);
            let then_predecessor_id = format_ident!("__then_predecessor_id_{}", id);

            let phi_inputs = if if_expr.else_branch.is_some() {
                let post_else_name = format_ident!("__post_else_{}_{}", name, id);
                let else_predecessor_id = format_ident!("__else_predecessor_id_{}", id);
                quote! { vec![(#then_predecessor_id, #post_then_name), (#else_predecessor_id, #post_else_name)] }
            } else {
                quote! { vec![(#then_predecessor_id, #post_then_name), (#parent_predecessor_id, #pre_if_name)] }
            };

            quote! {
                #name = graph::phi(&#builder, #phi_inputs);
            }
        });

        // --- 8. Assemble the final expression ---
        let then_predecessor_id = format_ident!("__then_predecessor_id_{}", id);
        parse_quote! {
            {
                #(#pre_if_captures)*

                let #merge_block_id = #builder.new_block();
                let #then_block_id = #builder.new_block();
                #else_block_setup

                let __if_condition = #cond;
                let #parent_predecessor_id = #builder.current_block_id();
                #builder.seal_block(graph::Terminator::branch(__if_condition, #then_block_id, #false_target));

                #builder.switch_to_block(#then_block_id);
                let __then_val = #then_branch_body;
                #(#then_post_captures)*
                let #then_predecessor_id = #builder.current_block_id();
                #builder.seal_block(graph::Terminator::jump(#merge_block_id));

                #else_block_impl

                #builder.switch_to_block(#merge_block_id);
                #(#merge_phis)*

                #phi
            }
        }
    }

    fn handle_while(&mut self, while_expr: &mut syn::ExprWhile) -> Expr {
        let id = self.new_control_flow_id();
        let builder = self.builder_ident.clone();
        let vars = self.list_vars();

        let header_block_id = format_ident!("__while_header_block_{}", id);
        let body_block_id = format_ident!("__while_body_block_{}", id);
        let exit_block_id = format_ident!("__while_exit_block_{}", id);
        let parent_predecessor_id = format_ident!("__while_parent_predecessor_{}", id);

        let pre_while_captures = vars.iter().map(|name| {
            let pre_while_name = format_ident!("__pre_while_{}_{}", name, id);
            quote! { let #pre_while_name = #name; }
        });

        let phi_node_creations = vars.iter().map(|name| {
            let phi_node_id = format_ident!("__{}_phi_node_id_{}", name, id);
            quote! {
                let #phi_node_id = #builder.arena_mut().new_node(graph::NodeKind::Phi { from: vec![] });
                #name = graph::TracedValue::new(#phi_node_id);
                #builder.add_instruction_to_current_block(#phi_node_id);
            }
        });

        self.visit_expr_mut(&mut while_expr.cond);
        let cond = &while_expr.cond;

        self.visit_block_mut(&mut while_expr.body);
        let body_block = &while_expr.body;

        let post_body_captures = vars.iter().map(|name| {
            let post_body_name = format_ident!("__post_body_{}_{}", name, id);
            quote! { let #post_body_name = #name; }
        });

        let body_predecessor_id = format_ident!("__body_predecessor_id_{}", id);

        let mut phi_patchers = proc_macro2::TokenStream::new();
        for var in &vars {
            let phi_node_id = format_ident!("__{}_phi_node_id_{}", var, id);
            let pre_while_name = format_ident!("__pre_while_{}_{}", var, id);
            let post_body_name = format_ident!("__post_body_{}_{}", var, id);
            phi_patchers.extend(quote! {
                if let Some(graph::Node { kind: graph::NodeKind::Phi { from }, .. })
                    = #builder.arena_mut().nodes.get_mut(#phi_node_id) {
                    *from = vec![
                        (#parent_predecessor_id, #pre_while_name.id),
                        (#body_predecessor_id, #post_body_name.id)
                    ];
                }
            });
        }

        let exit_phis = vars.iter().map(|name| {
            let phi_node_id = format_ident!("__{}_phi_node_id_{}", name, id);
            quote! {
                #name = graph::TracedValue::new(#phi_node_id);
            }
        });

        parse_quote! {
            {
                #(#pre_while_captures)*

                let #header_block_id = #builder.new_block();
                let #body_block_id = #builder.new_block();
                let #exit_block_id = #builder.new_block();

                let #parent_predecessor_id = #builder.current_block_id();
                #builder.seal_block(graph::Terminator::jump(#header_block_id));

                #builder.switch_to_block(#header_block_id);
                #(#phi_node_creations)*
                let __while_cond = #cond;
                #builder.seal_block(graph::Terminator::branch(__while_cond, #body_block_id, #exit_block_id));

                #builder.switch_to_block(#body_block_id);
                #body_block;
                #(#post_body_captures)*
                let #body_predecessor_id = #builder.current_block_id();
                #builder.seal_block(graph::Terminator::jump(#header_block_id));

                #phi_patchers

                #builder.switch_to_block(#exit_block_id);
                #(#exit_phis)*
            }
        }
    }
}

pub fn workflow(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let func = parse_macro_input!(item as ItemFn);
    let func_name = &func.sig.ident;
    let mut func_body = func.block.clone();

    let return_type = match &func.sig.output {
        ReturnType::Type(_, ty) => quote! { #ty },
        ReturnType::Default => quote! { () },
    };

    let builder_ident = format_ident!("__builder");
    let mut visitor = WorkflowVisitor::new(builder_ident.clone());
    visitor.enter_scope();
    visitor.visit_block_mut(&mut func_body);
    visitor.exit_scope();

    let body_and_seal = if visitor.last_expr_has_value {
        quote! {
            let __result = #func_body;
            #builder_ident.seal_block(graph::Terminator::return_value(__result.id));
        }
    } else {
        quote! {
            #func_body;
            #builder_ident.seal_block(graph::Terminator::return_unit());
        }
    };

    let expanded = quote! {
        #[allow(unused_assignments)]
        #[allow(unused_braces)]
        pub fn #func_name() -> graph::Graph<#return_type> {
            let #builder_ident = graph::Builder::<#return_type>::new();

            #body_and_seal

            #builder_ident.build()
        }
    };

    TokenStream::from(expanded)
}
