extern crate proc_macro;

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::visit_mut::{self, VisitMut};
use syn::{
    Block, Expr, ExprIf, Ident, ItemFn, Pat, ReturnType, Stmt, parse_macro_input, parse_quote,
};

#[proc_macro_attribute]
pub fn task(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let func = parse_macro_input!(item as ItemFn);
    let func_name = &func.sig.ident;
    let func_impl_name = format_ident!("__impl_{}", func_name);

    let original_func_renamed = {
        let mut f = func.clone();
        f.sig.ident = func_impl_name.clone();
        quote! {
            #[allow(clippy::all)]
            #f
        }
    };

    let mut constructor_sig = func.sig.clone();
    let mut arg_names = Vec::new();
    let mut arg_types = Vec::new();

    constructor_sig.inputs.clear();
    constructor_sig
        .inputs
        .push(parse_quote! { builder: &mut graph::Builder });
    for arg in func.sig.inputs.iter() {
        if let syn::FnArg::Typed(pt) = arg {
            let pat_ident = if let Pat::Ident(pi) = &*pt.pat {
                pi
            } else {
                panic!("Only simple idents are supported in task arguments");
            };
            arg_names.push(pat_ident.ident.clone());

            let name = &pt.pat;
            let ty = &pt.ty;
            arg_types.push(ty.clone());
            constructor_sig
                .inputs
                .push(parse_quote! { #name: graph::TracedValue<#ty> });
        }
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

struct SsaBuilder {
    scopes: Vec<std::collections::HashMap<Ident, (Ident, bool)>>, // name -> (ssa_name, is_mutable)
    next_var_id: usize,
    return_type: proc_macro2::TokenStream,
    builder_ident: Ident,
}

impl SsaBuilder {
    fn new(return_type: proc_macro2::TokenStream, builder_ident: Ident) -> Self {
        Self {
            scopes: vec![Default::default()],
            next_var_id: 0,
            return_type,
            builder_ident,
        }
    }

    fn new_ssa_name(&mut self, name: &Ident) -> Ident {
        let id = self.next_var_id;
        self.next_var_id += 1;
        format_ident!("__ssa_{}_{}", name, id)
    }

    fn enter_scope(&mut self) {
        self.scopes.push(Default::default());
    }

    fn exit_scope(&mut self) {
        self.scopes.pop();
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
            .insert(name, (ssa_name, is_mutable));
    }
}

impl VisitMut for SsaBuilder {
    fn visit_block_mut(&mut self, i: &mut Block) {
        self.enter_scope();
        let num_stmts = i.stmts.len();
        for (idx, stmt) in i.stmts.iter_mut().enumerate() {
            if idx == num_stmts - 1 {
                if let Stmt::Expr(expr, semi) = stmt {
                    if semi.is_none() {
                        self.visit_expr_mut(expr);
                        continue;
                    }
                }
            }
            self.visit_stmt_mut(stmt);
        }
        self.exit_scope();
    }

    fn visit_expr_mut(&mut self, i: &mut Expr) {
        match i {
            Expr::Call(call_expr) => {
                // Manually visit children first to perform SSA substitution on args
                self.visit_expr_mut(&mut call_expr.func);
                for arg in call_expr.args.iter_mut() {
                    self.visit_expr_mut(arg);
                }

                // Now, inject the builder argument.
                let builder_ident = &self.builder_ident;
                call_expr
                    .args
                    .insert(0, parse_quote! { &mut #builder_ident });
                return; // Prevent double-visiting
            }
            Expr::If(if_expr) => {
                // Manually visit children first
                self.visit_expr_mut(&mut if_expr.cond);
                self.visit_block_mut(&mut if_expr.then_branch);
                if let Some((_, else_expr)) = &mut if_expr.else_branch {
                    self.visit_expr_mut(else_expr);
                }

                // Now that children are rewritten, rewrite the if itself.
                *i = self.rewrite_if(if_expr);
                return; // Prevent default visit_mut recursion
            }
            Expr::Path(path) => {
                if let Some(ident) = path.path.get_ident() {
                    if let Some((ssa_var, _)) = self.get_var(ident) {
                        *i = parse_quote! { #ssa_var };
                    }
                }
            }
            _ => {}
        }
        visit_mut::visit_expr_mut(self, i);
    }

    fn visit_stmt_mut(&mut self, i: &mut Stmt) {
        if let Stmt::Local(local) = i {
            let pat_ident = if let Pat::Ident(pi) = &local.pat {
                pi
            } else {
                panic!("Only simple idents are supported in let bindings");
            };

            let name = pat_ident.ident.clone();
            let ssa_name = self.new_ssa_name(&name);
            let is_mutable = pat_ident.mutability.is_some();

            self.insert_var(name, ssa_name.clone(), is_mutable);

            if let Some(init) = &local.init {
                let mut init_clone = init.expr.clone();
                self.visit_expr_mut(&mut init_clone);

                *i = parse_quote! {
                    let #ssa_name = #init_clone;
                };
            } else {
                panic!("Let bindings must be initialized");
            }
            return;
        }

        if let Stmt::Expr(expr, _semi) = i {
            if let Expr::Assign(assign_expr) = expr {
                if let Expr::Path(path) = &*assign_expr.left {
                    if let Some(name) = path.path.get_ident() {
                        let var_info = self
                            .get_var(name)
                            .unwrap_or_else(|| panic!("Variable {} not found", name));

                        if !var_info.1 {
                            panic!("Cannot assign to immutable variable {}", name);
                        }

                        let mut rhs = assign_expr.right.clone();
                        self.visit_expr_mut(&mut rhs);

                        let ssa_name = self.new_ssa_name(name);
                        self.insert_var(name.clone(), ssa_name.clone(), true);

                        *i = parse_quote! {
                            let #ssa_name = #rhs;
                        };
                        return;
                    }
                }
            }
        }

        visit_mut::visit_stmt_mut(self, i);
    }
}

impl SsaBuilder {
    fn rewrite_if(&mut self, if_expr: &ExprIf) -> Expr {
        let cond = if_expr.cond.clone();
        let then_branch = if_expr.then_branch.clone();
        let else_branch = if let Some((_, else_expr)) = &if_expr.else_branch {
            quote! { {#else_expr} }
        } else {
            let builder = &self.builder_ident;
            quote! { graph::new_literal(&mut #builder, ()) }
        };

        let return_type = &self.return_type;
        let builder = &self.builder_ident;
        let new_expr = quote! {
            {
                let result: graph::TracedValue<#return_type> = {
                    let cond_val = #cond;

                    let then_block_id = #builder.new_block();
                    let else_block_id = #builder.new_block();
                    let merge_block_id = #builder.new_block();

                    #builder.seal_block(graph::Terminator::Branch {
                        condition: cond_val.id,
                        true_target: then_block_id,
                        false_target: else_block_id,
                    });

                    #builder.switch_to_block(then_block_id);

                    let then_val = #then_branch;

                    #builder.seal_block(graph::Terminator::Jump {
                        target: merge_block_id,
                    });
                    #builder.switch_to_block(else_block_id);

                    let else_val = #else_branch;

                    #builder.seal_block(graph::Terminator::Jump {
                        target: merge_block_id,
                    });
                    #builder.switch_to_block(merge_block_id);

                    let kind = graph::NodeKind::Phi {
                        from: vec![(then_block_id, then_val.id), (else_block_id, else_val.id)],
                    };
                    let id = #builder.add_instruction(kind);
                    graph::TracedValue::new(id)
                };
                result
            }
        };

        parse_quote! { #new_expr }
    }
}

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
    SsaBuilder::new(return_type.clone(), builder_ident.clone()).visit_block_mut(&mut func_body);

    let expanded = quote! {
        pub fn #func_name() -> graph::Graph {
            let mut #builder_ident = graph::Builder::new();

            let result_val: graph::TracedValue<#return_type> = #func_body;

            #builder_ident.seal_block(graph::Terminator::Return { value: result_val.id });
            #builder_ident.build()
        }
    };

    TokenStream::from(expanded)
}
