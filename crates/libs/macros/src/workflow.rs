use std::collections::BTreeSet;

use darling::FromMeta;
use darling::ast::NestedMeta;
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::punctuated::Punctuated;
use syn::visit_mut::{self, VisitMut};
use syn::{
    Block, Expr, ExprIf, ExprMatch, Ident, ItemFn, Pat, ReturnType, Stmt, Token, parse_macro_input,
    parse_quote,
};

struct WorkflowVisitor {
    // We use BTreeSet for deterministic ordering of variables.
    scopes: Vec<BTreeSet<Ident>>,
    next_control_flow_id: usize,
    builder_ident: Ident,
    last_expr_has_value: bool,
    /// The first unsupported construct found; reported instead of the expansion.
    error: Option<syn::Error>,
}

#[derive(Debug, Default, FromMeta)]
struct WorkflowArgs {
    id: Option<String>,
}

impl WorkflowVisitor {
    fn new(builder_ident: Ident) -> Self {
        Self {
            scopes: vec![],
            next_control_flow_id: 0,
            builder_ident,
            last_expr_has_value: false,
            error: None,
        }
    }

    fn fail(&mut self, tokens: &impl quote::ToTokens, message: &str) {
        if self.error.is_none() {
            self.error = Some(syn::Error::new_spanned(tokens, message));
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
        self.scopes.iter().flat_map(|x| x.iter().cloned()).collect()
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
                if matches!(&*call_expr.func, Expr::Path(func) if func.path.is_ident("attempt")) {
                    let call_expr = call_expr.clone();
                    self.fail(
                        &call_expr,
                        "an attempt is matched on: `match attempt(task(..)) { Ok(..) => .., Err(..) => .. }`",
                    );
                    return;
                }
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
                let expr = expr.clone();
                self.fail(
                    &expr,
                    "assignments are not supported in expressions, put semicolon after the assignment",
                );
            }
            Expr::If(if_expr) => {
                *i = self.handle_if(if_expr);
            }
            Expr::Match(match_expr) => {
                *i = self.handle_match(match_expr);
            }
            Expr::While(while_expr) => {
                *i = self.handle_while(while_expr);
                self.last_expr_has_value = false;
            }
            Expr::Return(return_expr) => {
                *i = self.handle_return(return_expr);
                self.last_expr_has_value = false;
            }
            Expr::Lit(lit) => {
                let builder_ident = &self.builder_ident;
                // Workflow values are owned, so string literals are `String`, matching task
                // parameters (a `&str` could never cross a task boundary anyway).
                *i = if matches!(lit.lit, syn::Lit::Str(_)) {
                    parse_quote! {
                        ::namu::__macro_exports::literal(&#builder_ident, ::std::string::String::from(#lit))
                    }
                } else {
                    parse_quote! { ::namu::__macro_exports::literal(&#builder_ident, #lit) }
                };
                self.last_expr_has_value = true;
            }
            _ => visit_mut::visit_expr_mut(self, i),
        }
    }

    fn visit_stmt_mut(&mut self, i: &mut Stmt) {
        match i {
            Stmt::Local(local) => {
                match &mut local.pat {
                    Pat::Ident(pat_ident) => {
                        if let Some(init) = &mut local.init {
                            let is_mut = pat_ident.mutability.is_some();
                            self.visit_expr_mut(&mut init.expr);
                            if is_mut {
                                self.insert_var(pat_ident.ident.clone());
                            }
                        } else {
                            let local = local.clone();
                            self.fail(&local, "let bindings must be initialized");
                        }
                    }
                    Pat::Tuple(pat_tuple) => {
                        if pat_tuple.elems.is_empty() {
                            let local = local.clone();
                            self.fail(&local, "Tuple destructuring requires at least one element");
                            return;
                        }
                        if let Some(init) = &mut local.init {
                            self.visit_expr_mut(&mut init.expr);
                        } else {
                            let local = local.clone();
                            self.fail(&local, "Tuple destructuring requires initializer");
                            return;
                        }

                        // Track mut variables inside tuple pattern
                        for elem in pat_tuple.elems.iter() {
                            if let Pat::Ident(pat_ident) = elem {
                                if pat_ident.mutability.is_some() {
                                    self.insert_var(pat_ident.ident.clone());
                                }
                            } else {
                                let elem = elem.clone();
                                self.fail(
                                    &elem,
                                    "Only simple idents are supported in tuple destructuring",
                                );
                            }
                        }
                    }
                    _ => {
                        let local = local.clone();
                        self.fail(
                            &local,
                            "only simple idents or tuple patterns are supported in let bindings",
                        );
                    }
                }

                self.last_expr_has_value = false;
            }
            Stmt::Expr(Expr::Assign(assign_expr), _semi) => {
                let Expr::Path(path) = &*assign_expr.left else {
                    let assign_expr = assign_expr.clone();
                    self.fail(
                        &assign_expr,
                        "only simple idents are supported in let bindings",
                    );
                    return;
                };
                let Some(name) = path.path.get_ident() else {
                    let path = path.clone();
                    self.fail(&path, "only simple idents are supported in let bindings");
                    return;
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
    /// `return` ends the branch: inside a fan-out it drops the item (design §4.2). What follows
    /// in the same block can never run, so it goes into a block nothing jumps to.
    fn handle_return(&mut self, return_expr: &mut syn::ExprReturn) -> Expr {
        let builder = self.builder_ident.clone();
        let seal = match &mut return_expr.expr {
            Some(value) => {
                self.visit_expr_mut(value);
                quote! { ::namu::__macro_exports::return_value(&#builder, #value); }
            }
            None => quote! { ::namu::__macro_exports::return_unit(&#builder); },
        };
        parse_quote! {
            {
                #seal
                let __unreachable = #builder.new_block();
                #builder.switch_to_block(__unreachable);
            }
        }
    }

    fn handle_if(&mut self, if_expr: &mut ExprIf) -> Expr {
        let builder = self.builder_ident.clone();
        let id = self.new_control_flow_id();
        let (then_block, else_block, merge_block) = two_way_blocks(id);

        self.visit_expr_mut(&mut if_expr.cond);
        let cond = &if_expr.cond;
        let false_target = if if_expr.else_branch.is_some() {
            else_block
        } else {
            merge_block
        };
        let evaluate = quote! { let __if_condition = #cond; };
        let seal = quote! {
            ::namu::__macro_exports::branch(&#builder, __if_condition, #then_block, #false_target);
        };
        let else_expr = if_expr.else_branch.as_mut().map(|(_, expr)| &mut **expr);
        self.two_way(id, [evaluate, seal], &mut if_expr.then_branch, else_expr)
    }

    /// `match attempt(task(..)) { Ok(v) => .., Err(e) => .. }`: the call gets an error edge
    /// (design §4.4). `v` is bound to its outputs where it succeeded, `e` to its error message
    /// where it failed.
    fn handle_match(&mut self, match_expr: &mut ExprMatch) -> Expr {
        const USAGE: &str =
            "only `match attempt(task(..)) { Ok(..) => .., Err(..) => .. }` is supported";
        let Some(call) = attempted(&match_expr.expr) else {
            self.fail(&match_expr.expr, USAGE);
            return Expr::Match(match_expr.clone());
        };
        let mut call = call.clone();
        let (mut ok, mut err) = (None, None);
        for arm in &match_expr.arms {
            let Some((variant, pat)) = result_arm(&arm.pat) else {
                self.fail(&arm.pat, USAGE);
                return Expr::Match(match_expr.clone());
            };
            let slot = if variant == "Ok" { &mut ok } else { &mut err };
            if slot.replace((pat.clone(), arm.body.clone())).is_some() {
                self.fail(&arm.pat, "each of `Ok` and `Err` takes one arm");
                return Expr::Match(match_expr.clone());
            }
        }
        let (Some((ok_pat, ok_body)), Some((err_pat, err_body))) = (ok, err) else {
            self.fail(&match_expr, "an attempt needs an `Ok` arm and an `Err` arm");
            return Expr::Match(match_expr.clone());
        };

        let builder = self.builder_ident.clone();
        let id = self.new_control_flow_id();
        let (then_block, else_block, _) = two_way_blocks(id);
        let value = format_ident!("__attempt_value_{}", id);
        let error = format_ident!("__attempt_error_{}", id);

        self.visit_expr_mut(&mut call);
        let evaluate = quote! { let #value = #call; };
        let seal = quote! {
            let #error = ::namu::__macro_exports::catch(&#builder, #then_block, #else_block);
        };
        let bind = |pat: &Pat, from: &Ident, body: &Expr| -> Block {
            match pat {
                Pat::Wild(_) => parse_quote!({ #body }),
                _ => parse_quote!({ let #pat = #from; #body }),
            }
        };
        let mut ok_block = bind(&ok_pat, &value, &ok_body);
        let mut err_expr = Expr::Block(syn::ExprBlock {
            attrs: Vec::new(),
            label: None,
            block: bind(&err_pat, &error, &err_body),
        });
        self.two_way(id, [evaluate, seal], &mut ok_block, Some(&mut err_expr))
    }

    /// Lowers a two-way split: `evaluate` (already visited) computes what decides it, which may
    /// itself add blocks, and `seal` ends the block that leaves towards the blocks of
    /// [`two_way_blocks`]; then `then` runs, or `else_expr` (or nothing), and both meet in the
    /// merge block, where every variable and the value of the whole gets a phi.
    fn two_way(
        &mut self,
        id: usize,
        [evaluate, seal]: [proc_macro2::TokenStream; 2],
        then: &mut Block,
        else_expr: Option<&mut Expr>,
    ) -> Expr {
        let builder = self.builder_ident.clone();
        let vars = self.list_vars();
        let (then_block_id, else_block_id, merge_block_id) = two_way_blocks(id);

        let pre_if_captures = vars.iter().map(|name| {
            let pre_if_name = format_ident!("__pre_if_{}_{}", name, id);
            quote! { let #pre_if_name = #name; }
        });

        let parent_predecessor_id = format_ident!("__if_parent_predecessor_{}", id);

        self.visit_block_mut(then);
        let then_branch_body = &*then;
        let then_has_value = self.last_expr_has_value;

        let then_post_captures = vars.iter().map(|name| {
            let post_then_name = format_ident!("__post_then_{}_{}", name, id);
            quote! { let #post_then_name = #name; }
        });

        let has_else = else_expr.is_some();
        let (else_block_setup, else_block_impl, phi) = if let Some(else_expr) = else_expr {
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
                ::namu::__macro_exports::jump(&#builder, #merge_block_id);
            };

            let then_predecessor_id = format_ident!("__then_predecessor_id_{}", id);
            let phi = if then_has_value && else_has_value {
                Some(quote! {
                     ::namu::__macro_exports::phi(
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
            self.last_expr_has_value = phi.is_some();

            (Some(setup), Some(implementation), phi)
        } else {
            self.last_expr_has_value = false;
            (None, None, None)
        };

        let merge_phis = vars.iter().map(|name| {
            let pre_if_name = format_ident!("__pre_if_{}_{}", name, id);
            let post_then_name = format_ident!("__post_then_{}_{}", name, id);
            let then_predecessor_id = format_ident!("__then_predecessor_id_{}", id);

            let phi_inputs = if has_else {
                let post_else_name = format_ident!("__post_else_{}_{}", name, id);
                let else_predecessor_id = format_ident!("__else_predecessor_id_{}", id);
                quote! { vec![(#then_predecessor_id, #post_then_name), (#else_predecessor_id, #post_else_name)] }
            } else {
                quote! { vec![(#then_predecessor_id, #post_then_name), (#parent_predecessor_id, #pre_if_name)] }
            };

            quote! {
                #name = ::namu::__macro_exports::phi(&#builder, #phi_inputs);
            }
        });

        let then_predecessor_id = format_ident!("__then_predecessor_id_{}", id);
        parse_quote! {
            {
                #(#pre_if_captures)*

                let #merge_block_id = #builder.new_block();
                let #then_block_id = #builder.new_block();
                #else_block_setup

                #evaluate
                let #parent_predecessor_id = #builder.current_block_id();
                #seal

                #builder.switch_to_block(#then_block_id);
                let __then_val = #then_branch_body;
                #(#then_post_captures)*
                let #then_predecessor_id = #builder.current_block_id();
                ::namu::__macro_exports::jump(&#builder, #merge_block_id);

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
            let phi_val_ident = format_ident!("__{}_phi_val_{}", name, id);
            let pre_while_name = format_ident!("__pre_while_{}_{}", name, id);
            quote! {
                let #phi_val_ident = {
                    let __phi_id = #builder.phi(vec![(#parent_predecessor_id, #pre_while_name.id)]);
                    ::namu::__macro_exports::TracedValue::new(__phi_id)
                };
                #name = #phi_val_ident;
                let #phi_node_id = #builder.arena().nodes.len() - 1;
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
                if let Some(::namu::__macro_exports::Node { kind: ::namu::__macro_exports::NodeKind::Phi { from }, .. })
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
                #name = ::namu::__macro_exports::TracedValue::new(#phi_node_id);
            }
        });

        parse_quote! {
            {
                #(#pre_while_captures)*

                let #header_block_id = #builder.new_block();
                let #body_block_id = #builder.new_block();
                let #exit_block_id = #builder.new_block();

                let #parent_predecessor_id = #builder.current_block_id();
                ::namu::__macro_exports::jump(&#builder, #header_block_id);

                #builder.switch_to_block(#header_block_id);
                #(#phi_node_creations)*
                let __while_cond = #cond;
                ::namu::__macro_exports::branch(&#builder, __while_cond, #body_block_id, #exit_block_id);

                #builder.switch_to_block(#body_block_id);
                #body_block;
                #(#post_body_captures)*
                let #body_predecessor_id = #builder.current_block_id();
                ::namu::__macro_exports::jump(&#builder, #header_block_id);

                #phi_patchers

                #builder.switch_to_block(#exit_block_id);
                #(#exit_phis)*
            }
        }
    }
}

/// The blocks of a two-way split: then, else, and where they meet.
fn two_way_blocks(id: usize) -> (Ident, Ident, Ident) {
    (
        format_ident!("__if_then_block_{}", id),
        format_ident!("__if_else_block_{}", id),
        format_ident!("__if_merge_block_{}", id),
    )
}

/// The call in `attempt(call)`.
fn attempted(expr: &Expr) -> Option<&Expr> {
    let Expr::Call(outer) = expr else {
        return None;
    };
    let Expr::Path(func) = &*outer.func else {
        return None;
    };
    if !func.path.is_ident("attempt") || outer.args.len() != 1 {
        return None;
    }
    let call = &outer.args[0];
    matches!(call, Expr::Call(_)).then_some(call)
}

/// `Ok(pat)` or `Err(pat)`: the variant and the pattern inside.
fn result_arm(pat: &Pat) -> Option<(&'static str, &Pat)> {
    let Pat::TupleStruct(pat) = pat else {
        return None;
    };
    let variant = ["Ok", "Err"]
        .into_iter()
        .find(|variant| pat.path.is_ident(variant))?;
    match pat.elems.len() {
        1 => Some((variant, &pat.elems[0])),
        _ => None,
    }
}

pub fn workflow(attr: TokenStream, item: TokenStream) -> TokenStream {
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
    if let Some(err) = visitor.error {
        return err.to_compile_error().into();
    }

    let args = if attr.is_empty() {
        WorkflowArgs::default()
    } else {
        let meta: Punctuated<NestedMeta, Token![,]> =
            parse_macro_input!(attr with Punctuated::<NestedMeta, Token![,]>::parse_terminated);
        let meta_vec: Vec<NestedMeta> = meta.into_iter().collect();
        match WorkflowArgs::from_list(&meta_vec) {
            Ok(v) => v,
            Err(e) => return e.write_errors().into(),
        }
    };
    let workflow_id = args.id.unwrap_or_else(|| func_name.to_string());
    let build_ident = format_ident!("__namu_build_{}", func_name);

    let body_and_seal = if visitor.last_expr_has_value {
        quote! {
            let __result = #func_body;
            ::namu::__macro_exports::return_value(&#builder_ident, __result);
        }
    } else {
        quote! {
            #func_body;
            ::namu::__macro_exports::return_unit(&#builder_ident);
        }
    };

    let expanded = quote! {
        #[allow(unused_assignments)]
        #[allow(unused_braces)]
        pub fn #func_name() -> ::namu::__macro_exports::Graph<#return_type> {
            let #builder_ident = ::namu::__macro_exports::Builder::<#return_type>::new();

            #body_and_seal

            #builder_ident.build()
        }

        fn #build_ident() -> ::namu::__macro_exports::Workflow {
            #func_name().to_serializable(#workflow_id.to_string())
        }

        ::namu::__macro_exports::inventory::submit! {
            ::namu::__macro_exports::WorkflowEntry {
                id: #workflow_id,
                build: #build_ident,
            }
        }
    };

    TokenStream::from(expanded)
}
