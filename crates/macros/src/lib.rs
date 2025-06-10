use std::collections::HashMap;

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
        .push(parse_quote! { builder: &mut graph::Builder<T> });

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
            let id = builder.add_instruction(kind);
            graph::TracedValue::new(id)
        }
    };

    quote! {
        #original_func_renamed
        #constructor
    }
    .into()
}

#[derive(Default, Debug, Clone)]
struct Scope {
    vars: HashMap<Ident, bool>, // name -> is_mutable
}

impl Scope {
    fn get(&self, name: &Ident) -> Option<bool> {
        self.vars.get(name).cloned()
    }

    fn insert(&mut self, name: Ident, is_mutable: bool) {
        self.vars.insert(name, is_mutable);
    }
}

struct WorkflowVisitor {
    scopes: Vec<Scope>,
    next_if_block_id: usize,
    builder_ident: Ident,
}

impl WorkflowVisitor {
    fn new(builder_ident: Ident) -> Self {
        Self {
            scopes: vec![],
            next_if_block_id: 0,
            builder_ident,
        }
    }

    fn new_if_id(&mut self) -> usize {
        let id = self.next_if_block_id;
        self.next_if_block_id += 1;
        id
    }

    fn enter_scope(&mut self) {
        self.scopes.push(Default::default());
    }

    fn exit_scope(&mut self) -> Scope {
        self.scopes.pop().unwrap()
    }

    fn get_var(&self, name: &Ident) -> Option<bool> {
        for scope in self.scopes.iter().rev() {
            if let Some(is_mut) = scope.get(name) {
                return Some(is_mut);
            }
        }
        None
    }

    fn insert_var(&mut self, name: Ident, is_mutable: bool) {
        self.scopes.last_mut().unwrap().insert(name, is_mutable);
    }
}

impl VisitMut for WorkflowVisitor {
    fn visit_block_mut(&mut self, i: &mut Block) {
        let has_return_expr = if let Some(last_stmt) = i.stmts.last() {
            matches!(last_stmt, Stmt::Expr(_, None))
        } else {
            false
        };

        for stmt in i.stmts.iter_mut() {
            self.visit_stmt_mut(stmt);
        }

        if !has_return_expr {
            let builder_ident = &self.builder_ident;
            let new_expr: Expr = parse_quote! {
                graph::new_literal(&mut #builder_ident, ())
            };
            i.stmts.push(Stmt::Expr(new_expr, None));
        }
    }

    fn visit_expr_block_mut(&mut self, i: &mut syn::ExprBlock) {
        self.enter_scope();
        visit_mut::visit_expr_block_mut(self, i);
        self.exit_scope();
    }

    fn visit_expr_mut(&mut self, i: &mut Expr) {
        match i {
            Expr::Call(call_expr) => {
                for arg in call_expr.args.iter_mut() {
                    self.visit_expr_mut(arg);
                }

                let builder_ident = &self.builder_ident;
                call_expr
                    .args
                    .insert(0, parse_quote! { &mut #builder_ident });
            }
            Expr::Path(path) => {
                if let Some(ident) = path.path.get_ident() {
                    if self.get_var(ident).is_none() {
                        abort!(ident, "cannot find value `{}` in this scope", ident);
                    }
                }
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
            Expr::Lit(lit) => {
                let builder_ident = &self.builder_ident;
                *i = parse_quote! { graph::new_literal(&mut #builder_ident, #lit) };
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
                    self.insert_var(pat_ident.ident.clone(), is_mut);
                } else {
                    abort!(local, "let bindings must be initialized");
                }
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

                let Some(is_mut) = self.get_var(name) else {
                    abort!(name, "cannot find value `{}` in this scope", name);
                };

                if !is_mut {
                    abort!(name, "cannot assign twice to immutable variable `{}`", name);
                }

                self.visit_expr_mut(&mut assign_expr.right);
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

        // --- 1. Identify mutable variables in scope before the `if` ---
        let mut mutable_vars = HashMap::new();
        for scope in self.scopes.iter().rev() {
            for (name, &is_mutable) in &scope.vars {
                if is_mutable && !mutable_vars.contains_key(name) {
                    mutable_vars.insert(name.clone(), name.clone());
                }
            }
        }

        // --- 2. Create pre-capture statements for mutable vars ---
        let pre_if_captures = mutable_vars.keys().map(|name| {
            let pre_if_name = format_ident!("__pre_if_{}_{}", name, if_id);
            quote! { let #pre_if_name = #name; }
        });

        // --- 3. Visit condition ---
        self.visit_expr_mut(&mut if_expr.cond);
        let cond = &if_expr.cond;

        // --- 4. Set up block IDs ---
        let then_block_id = format_ident!("__if_then_block_{}", if_id);
        let merge_block_id = format_ident!("__if_merge_block_{}", if_id);
        let parent_predecessor_id = format_ident!("__if_parent_predecessor_{}", if_id);

        // --- 5. Process `then` branch ---
        self.enter_scope();
        self.visit_block_mut(&mut if_expr.then_branch);
        self.exit_scope();
        let then_branch_body = &if_expr.then_branch;

        let then_post_captures = mutable_vars.keys().map(|name| {
            let post_then_name = format_ident!("__post_then_{}_{}", name, if_id);
            quote! { let #post_then_name = #name; }
        });

        // --- 6. Process `else` branch ---
        let (else_block_setup, else_block_impl, false_target, return_phi) =
            if let Some((_, else_expr)) = &mut if_expr.else_branch {
                let else_block_id = format_ident!("__if_else_block_{}", if_id);

                // --- reset mutable vars for the `else` branch ---
                let else_resets = mutable_vars.keys().map(|name| {
                    let pre_if_name = format_ident!("__pre_if_{}_{}", name, if_id);
                    quote! { #name = #pre_if_name; }
                });

                self.enter_scope();
                self.visit_expr_mut(else_expr);
                self.exit_scope();

                let else_post_captures = mutable_vars.keys().map(|name| {
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
                    let #else_predecessor_id = #builder.current_block_id;
                    #builder.seal_block(graph::Terminator::Jump { target: #merge_block_id });
                };

                let then_predecessor_id = format_ident!("__then_predecessor_id_{}", if_id);
                let phi = quote! {
                     graph::phi(
                        &mut #builder,
                        vec![
                            (#then_predecessor_id, __then_val),
                            (#else_predecessor_id, __else_val)
                        ]
                    )
                };

                (Some(setup), Some(implementation), else_block_id, phi)
            } else {
                let phi = quote! { graph::new_literal(&mut #builder, ()) };
                (None, None, merge_block_id.clone(), phi)
            };

        // --- 7. Create phi nodes for all mutable variables ---
        let merge_phis = mutable_vars.keys().map(|name| {
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
                #name = graph::phi(&mut #builder, #phi_inputs);
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
                let #parent_predecessor_id = #builder.current_block_id;
                #builder.seal_block(graph::Terminator::Branch {
                    condition: __if_condition.id,
                    true_target: #then_block_id,
                    false_target: #false_target,
                });

                #builder.switch_to_block(#then_block_id);
                let __then_val = #then_branch_body;
                #(#then_post_captures)*
                let #then_predecessor_id = #builder.current_block_id;
                #builder.seal_block(graph::Terminator::Jump { target: #merge_block_id });

                #else_block_impl

                #builder.switch_to_block(#merge_block_id);
                #(#merge_phis)*

                #return_phi
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

    let expanded = quote! {
        pub fn #func_name() -> graph::Graph<#return_type> {
            let mut #builder_ident = graph::Builder::<#return_type>::new();

            let result_val = #func_body;

            #builder_ident.seal_block(graph::Terminator::Return { value: result_val.id });
            #builder_ident.build()
        }
    };

    TokenStream::from(expanded)
}
