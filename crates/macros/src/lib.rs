use std::collections::HashSet;

use proc_macro::TokenStream;
use proc_macro_error::{abort, proc_macro_error};
use quote::{format_ident, quote};
use syn::visit_mut::{self, VisitMut};
use syn::{
    Block, Expr, ExprIf, Ident, ItemFn, Pat, ReturnType, Stmt, parse_macro_input, parse_quote,
};

#[proc_macro_error]
#[proc_macro_attribute]
pub fn task(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let func = parse_macro_input!(item as ItemFn);
    let func_name = &func.sig.ident;
    let func_impl_name = format_ident!("__impl_{}", func_name);

    let original_func_renamed = {
        let mut f = func.clone();
        f.sig.ident = func_impl_name.clone();
        quote! {
            #f
        }
    };

    let mut constructor_sig = func.sig.clone();
    let mut arg_names = Vec::new();
    let mut arg_types = Vec::new();

    constructor_sig.generics = parse_quote! { <T: Clone + 'static> };

    constructor_sig.inputs.clear();
    constructor_sig
        .inputs
        .push(parse_quote! { builder: &graph::Builder<T> });

    for arg in func.sig.inputs.iter() {
        let syn::FnArg::Typed(pt) = arg else {
            abort!(arg, "`self` is not supported");
        };
        let Pat::Ident(pat_ident) = &*pt.pat else {
            abort!(pt, "only simple idents are supported in task arguments");
        };
        let name = &pt.pat;
        let ty = &pt.ty;

        arg_names.push(pat_ident.ident.clone());
        arg_types.push(ty.clone());
        constructor_sig
            .inputs
            .push(parse_quote! { #name: graph::TracedValue<#ty> });
    }

    let return_type = match &func.sig.output {
        ReturnType::Type(_, ty) => quote! { #ty },
        ReturnType::Default => quote! { () },
    };
    constructor_sig.output = parse_quote! { -> graph::TracedValue<#return_type> };

    let value_downcasts = arg_names.iter().enumerate().map(|(i, name)| {
        let ty = &arg_types[i];
        quote! {
            let #name = __inputs[#i].downcast_ref::<#ty>().unwrap().clone();
        }
    });

    let input_ids = arg_names.iter().map(|name| quote! { #name.id });

    let constructor = quote! {
        #[allow(non_snake_case)]
        pub #constructor_sig {
            let func: graph::Executable = std::sync::Arc::new(|__inputs| {
                #(#value_downcasts)*
                let result = #func_impl_name(#(#arg_names),*);
                std::sync::Arc::new(result) as graph::Value
            });

            let kind = graph::NodeKind::Call {
                name: stringify!(#func_name),
                func,
                inputs: vec![#(#input_ids),*],
            };
            builder.add_instruction(kind)
        }
    };

    quote! {
        #original_func_renamed
        #constructor
    }
    .into()
}

struct WorkflowVisitor {
    scopes: Vec<HashSet<Ident>>,
    next_if_block_id: usize,
    next_while_block_id: usize,
    builder_ident: Ident,
    last_expr_has_value: bool,
}

impl WorkflowVisitor {
    fn new(builder_ident: Ident) -> Self {
        Self {
            scopes: vec![],
            next_if_block_id: 0,
            next_while_block_id: 0,
            builder_ident,
            last_expr_has_value: false,
        }
    }

    fn new_if_id(&mut self) -> usize {
        let id = self.next_if_block_id;
        self.next_if_block_id += 1;
        id
    }

    fn new_while_id(&mut self) -> usize {
        let id = self.next_while_block_id;
        self.next_while_block_id += 1;
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
                // if let Some(ident) = path.path.get_ident() {
                //     if self.get_var(ident).is_none() {
                //         abort!(ident, "cannot find value `{}` in this scope", ident);
                //     }
                // }

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
        let if_id = self.new_if_id();

        let vars = self.list_vars();

        self.visit_expr_mut(&mut if_expr.cond);
        let cond = &if_expr.cond;

        let pre_if_captures = vars.iter().map(|name| {
            let pre_if_name = format_ident!("__pre_if_{}_{}", name, if_id);
            quote! { let #pre_if_name = #name; }
        });

        let then_block_id = format_ident!("__if_then_block_{}", if_id);
        let merge_block_id = format_ident!("__if_merge_block_{}", if_id);
        let parent_predecessor_id = format_ident!("__if_parent_predecessor_{}", if_id);

        self.visit_block_mut(&mut if_expr.then_branch);
        let then_branch_body = &if_expr.then_branch;
        let then_has_value = self.last_expr_has_value;

        let then_post_captures = vars.iter().map(|name| {
            let post_then_name = format_ident!("__post_then_{}_{}", name, if_id);
            quote! { let #post_then_name = #name; }
        });

        let (else_block_setup, else_block_impl, false_target, phi) =
            if let Some((_, else_expr)) = &mut if_expr.else_branch {
                let else_block_id = format_ident!("__if_else_block_{}", if_id);

                let else_resets = vars.iter().map(|name| {
                    let pre_if_name = format_ident!("__pre_if_{}_{}", name, if_id);
                    quote! { #name = #pre_if_name; }
                });

                self.visit_expr_mut(else_expr);
                let else_has_value = self.last_expr_has_value;

                let else_post_captures = vars.iter().map(|name| {
                    let post_else_name = format_ident!("__post_else_{}_{}", name, if_id);
                    quote! { let #post_else_name = #name; }
                });

                let setup = quote! { let #else_block_id = #builder.new_block(); };
                let else_predecessor_id = format_ident!("__else_predecessor_id_{}", if_id);
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

                let then_predecessor_id = format_ident!("__then_predecessor_id_{}", if_id);
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
            let pre_if_name = format_ident!("__pre_if_{}_{}", name, if_id);
            let post_then_name = format_ident!("__post_then_{}_{}", name, if_id);
            let then_predecessor_id = format_ident!("__then_predecessor_id_{}", if_id);

            let phi_inputs = if if_expr.else_branch.is_some() {
                let post_else_name = format_ident!("__post_else_{}_{}", name, if_id);
                let else_predecessor_id = format_ident!("__else_predecessor_id_{}", if_id);
                quote! { vec![(#then_predecessor_id, #post_then_name), (#else_predecessor_id, #post_else_name)] }
            } else {
                quote! { vec![(#then_predecessor_id, #post_then_name), (#parent_predecessor_id, #pre_if_name)] }
            };

            quote! {
                #name = graph::phi(&#builder, #phi_inputs);
            }
        });

        // --- 8. Assemble the final expression ---
        let then_predecessor_id = format_ident!("__then_predecessor_id_{}", if_id);
        parse_quote! {
            {
                #(#pre_if_captures)*

                let #then_block_id = #builder.new_block();
                #else_block_setup
                let #merge_block_id = #builder.new_block();

                let __if_condition = #cond;
                let #parent_predecessor_id = #builder.current_block_id();
                #builder.seal_block(graph::Terminator::branch(__if_condition.id, #then_block_id, #false_target));

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
        let while_id = self.new_while_id();
        let builder = self.builder_ident.clone();
        let vars = self.list_vars();

        let header_block_id = format_ident!("__while_header_block_{}", while_id);
        let body_block_id = format_ident!("__while_body_block_{}", while_id);
        let exit_block_id = format_ident!("__while_exit_block_{}", while_id);
        let parent_predecessor_id = format_ident!("__while_parent_predecessor_{}", while_id);

        let pre_while_captures = vars.iter().map(|name| {
            let pre_while_name = format_ident!("__pre_while_{}_{}", name, while_id);
            quote! { let #pre_while_name = #name; }
        });

        let phi_node_creations = vars.iter().map(|name| {
            let phi_node_id = format_ident!("__{}_phi_node_id_{}", name, while_id);
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
            let post_body_name = format_ident!("__post_body_{}_{}", name, while_id);
            quote! { let #post_body_name = #name; }
        });

        let body_predecessor_id = format_ident!("__body_predecessor_id_{}", while_id);

        let mut phi_patchers = proc_macro2::TokenStream::new();
        for var in &vars {
            let phi_node_id = format_ident!("__{}_phi_node_id_{}", var, while_id);
            let pre_while_name = format_ident!("__pre_while_{}_{}", var, while_id);
            let post_body_name = format_ident!("__post_body_{}_{}", var, while_id);
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
            let phi_node_id = format_ident!("__{}_phi_node_id_{}", name, while_id);
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
                #builder.seal_block(graph::Terminator::branch(__while_cond.id, #body_block_id, #exit_block_id));

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

#[proc_macro_error]
#[proc_macro_attribute]
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

    let expanded = if visitor.last_expr_has_value {
        quote! {
            pub fn #func_name() -> graph::Graph<#return_type> {
                let #builder_ident = graph::Builder::<#return_type>::new();

                let __result = #func_body;

                #builder_ident.seal_block(graph::Terminator::return_value(__result.id));
                #builder_ident.build()
            }
        }
    } else {
        quote! {
            pub fn #func_name() -> graph::Graph<#return_type> {
                let #builder_ident = graph::Builder::<#return_type>::new();

                #func_body;

                #builder_ident.seal_block(graph::Terminator::return_unit());
                #builder_ident.build()
            }
        }
    };

    TokenStream::from(expanded)
}
