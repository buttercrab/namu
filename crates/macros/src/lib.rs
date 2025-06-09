extern crate proc_macro;

use std::collections::HashMap;

use proc_macro::TokenStream;
use proc_macro_error::{abort, proc_macro_error};
use quote::{format_ident, quote};
use syn::visit_mut::{self, VisitMut};
use syn::{
    Block, Expr, ExprIf, ExprWhile, Ident, ItemFn, Pat, ReturnType, Stmt, parse_macro_input,
    parse_quote,
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
            abort!(pt, "Only simple idents are supported in task arguments");
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

#[derive(Default, Debug)]
struct Scope {
    vars: HashMap<Ident, (Ident, bool)>, // name -> (ssa_name, is_mutable)
}

impl Scope {
    fn get(&self, name: &Ident) -> Option<(Ident, bool)> {
        self.vars.get(name).cloned()
    }

    fn insert(&mut self, name: Ident, ssa_name: Ident, is_mutable: bool) {
        self.vars.insert(name, (ssa_name, is_mutable));
    }

    fn merge(lhs: Scope, rhs: Scope) -> Scope {
        let mut vars = lhs.vars;
        vars.extend(rhs.vars);
        Scope { vars }
    }
}

struct SsaBuilder {
    scopes: Vec<Scope>,
    next_var_id: usize,
    next_if_block_id: usize,
    builder_ident: Ident,
}

impl SsaBuilder {
    fn new(builder_ident: Ident) -> Self {
        Self {
            scopes: vec![Default::default()],
            next_var_id: 0,
            next_if_block_id: 0,
            builder_ident,
        }
    }

    fn new_ssa_name(&mut self, name: &Ident) -> Ident {
        let id = self.next_var_id;
        self.next_var_id += 1;
        format_ident!("__ssa_{}_{}", name, id)
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

    fn get_var(&self, name: &Ident) -> Option<(Ident, bool)> {
        for scope in self.scopes.iter().rev() {
            if let Some(var_info) = scope.get(name) {
                return Some(var_info.clone());
            }
        }
        None
    }

    fn insert_var(&mut self, name: Ident, ssa_name: Ident, is_mutable: bool) {
        self.scopes
            .last_mut()
            .unwrap()
            .insert(name, ssa_name, is_mutable);
    }
}

impl VisitMut for SsaBuilder {
    fn visit_block_mut(&mut self, i: &mut Block) {
        let has_return_expr = if let Some(last_stmt) = i.stmts.last() {
            matches!(last_stmt, Stmt::Expr(_, None))
        } else {
            false
        };

        self.enter_scope();
        for stmt in i.stmts.iter_mut() {
            self.visit_stmt_mut(stmt);
        }
        self.exit_scope();

        if !has_return_expr {
            let builder_ident = &self.builder_ident;
            let new_expr: Expr = parse_quote! {
                graph::new_literal(&mut #builder_ident, ())
            };
            i.stmts.push(Stmt::Expr(new_expr, None));
        }
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
                let Some(ident) = path.path.get_ident() else {
                    abort!(path, "Only simple idents are supported in let bindings");
                };
                let Some((ssa_var, _)) = self.get_var(ident) else {
                    abort!(ident, "Variable '{}' not found", ident);
                };

                *i = parse_quote! { #ssa_var };
            }
            Expr::Assign(expr) => {
                abort!(
                    expr,
                    "Assignments are not supported in expressions, put semicolon after the assignment"
                );
            }
            Expr::If(if_expr) => {
                // Manually visit children first
                self.visit_expr_mut(&mut if_expr.cond);
                self.visit_block_mut(&mut if_expr.then_branch);
                if let Some((_, else_expr)) = &mut if_expr.else_branch {
                    self.visit_expr_mut(else_expr);
                }

                *i = self.handle_if(if_expr);
            }
            Expr::While(while_expr) => {
                self.visit_expr_mut(&mut while_expr.cond);
                self.visit_block_mut(&mut while_expr.body);

                *i = self.handle_while(while_expr);
            }
            _ => visit_mut::visit_expr_mut(self, i),
        }
    }

    fn visit_stmt_mut(&mut self, i: &mut Stmt) {
        match i {
            Stmt::Local(local) => {
                let Pat::Ident(pat_ident) = &local.pat else {
                    abort!(local, "Only simple idents are supported in let bindings");
                };
                let Some(mut init) = local.init.as_ref().cloned() else {
                    abort!(local, "Let bindings must be initialized");
                };

                // TODO: change is_mut
                let ssa_name = self.handle_assign(&pat_ident.ident, &mut init.expr, true, true);
                let rhs = init.expr;
                *i = parse_quote! { let #ssa_name = #rhs; };
            }
            Stmt::Expr(Expr::Assign(assign_expr), _semi) => {
                let Expr::Path(path) = &*assign_expr.left else {
                    abort!(
                        assign_expr,
                        "Only simple idents are supported in let bindings"
                    );
                };
                let Some(name) = path.path.get_ident() else {
                    abort!(path, "Only simple idents are supported in let bindings");
                };
                let ssa_name = self.handle_assign(name, &mut assign_expr.right, false, true);
                let rhs = &assign_expr.right;

                *i = parse_quote! { let #ssa_name = #rhs; };
            }
            Stmt::Expr(expr, _semi) => self.visit_expr_mut(expr),
            _ => visit_mut::visit_stmt_mut(self, i),
        }
    }
}

impl SsaBuilder {
    fn handle_assign(&mut self, name: &Ident, rhs: &mut Expr, new: bool, is_mut: bool) -> Ident {
        if !new {
            let Some((_, is_mut)) = self.get_var(name) else {
                abort!(name, "Variable '{}' not found", name);
            };

            if !is_mut {
                abort!(name, "Cannot assign to immutable variable '{}'", name);
            }
        }

        self.visit_expr_mut(rhs);

        let ssa_name = self.new_ssa_name(name);
        self.insert_var(name.clone(), ssa_name.clone(), is_mut);

        ssa_name
    }

    fn handle_if(&mut self, if_expr: &ExprIf) -> Expr {
        let builder = self.builder_ident.clone();

        let cond = &if_expr.cond;
        let then_branch = &if_expr.then_branch;

        let if_id = self.new_if_id();
        let then_block_id = format_ident!("__if_then_block_{}", if_id);
        let merge_block_id = format_ident!("__if_merge_block_{}", if_id);
        let then_predecessor_id = format_ident!("__if_then_predecessor_{}", if_id);
        let then_val = format_ident!("__if_then_val_{}", if_id);

        let (else_block_setup, else_block_impl, false_target, phi_call) =
            if let Some((_, else_expr)) = &if_expr.else_branch {
                // Case 1: `if-else` expression
                let else_block_id = format_ident!("__if_else_block_{}", if_id);
                let else_predecessor_id = format_ident!("__if_else_predecessor_{}", if_id);
                let else_val = format_ident!("__if_else_val_{}", if_id);

                let setup = quote! { let #else_block_id = #builder.new_block(); };
                let implementation = quote! {
                    #builder.switch_to_block(#else_block_id);
                    let #else_val = #else_expr;
                    let #else_predecessor_id = #builder.current_block_id;
                    #builder.seal_block(graph::Terminator::Jump {
                        target: #merge_block_id,
                    });
                };
                let phi = quote! {
                    graph::phi(
                        &mut #builder,
                        vec![
                            (#then_predecessor_id, #then_val),
                            (#else_predecessor_id, #else_val),
                        ],
                    )
                };

                (Some(setup), Some(implementation), else_block_id, Some(phi))
            } else {
                // Case 2: `if` expression without `else`. The value is always `()`.
                let false_target = merge_block_id.clone();

                (None, None, false_target, None)
            };

        parse_quote! {
            {
                let #then_block_id = #builder.new_block();
                #else_block_setup
                let #merge_block_id = #builder.new_block();

                let __if_condition = #cond;
                #builder.seal_block(graph::Terminator::Branch {
                    condition: __if_condition.id,
                    true_target: #then_block_id,
                    false_target: #false_target,
                });

                #builder.switch_to_block(#then_block_id);
                let #then_val = #then_branch;
                let #then_predecessor_id = #builder.current_block_id;
                #builder.seal_block(graph::Terminator::Jump {
                    target: #merge_block_id,
                });

                #else_block_impl

                #builder.switch_to_block(#merge_block_id);

                #phi_call
            }
        }
    }

    fn handle_while(&mut self, while_expr: &ExprWhile) -> Expr {
        unimplemented!()
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
    SsaBuilder::new(builder_ident.clone()).visit_block_mut(&mut func_body);

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
