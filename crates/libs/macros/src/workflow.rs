use std::collections::BTreeSet;

use darling::FromMeta;
use darling::ast::NestedMeta;
use proc_macro::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use syn::punctuated::Punctuated;
use syn::visit_mut::{self, VisitMut};
use syn::{
    Block, Expr, ExprIf, ExprMatch, FnArg, Ident, ItemFn, Pat, PatIdent, ReturnType, Stmt, Token,
    parse_macro_input, parse_quote,
};

struct WorkflowVisitor {
    // We use BTreeSet for deterministic ordering of variables.
    scopes: Vec<BTreeSet<Ident>>,
    next_control_flow_id: usize,
    builder_ident: Ident,
    last_expr_has_value: bool,
    /// The loops being lowered, innermost last: their ids and the variables their phis carry.
    loops: Vec<(usize, Vec<Ident>)>,
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
            loops: Vec::new(),
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
            Expr::Call(call_expr) if matches!(&*call_expr.func, Expr::Path(func) if func.path.is_ident("window")) =>
            {
                *i = self.handle_window(call_expr);
            }
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
            Expr::Break(break_expr) => {
                if break_expr.label.is_some() || break_expr.expr.is_some() {
                    let break_expr = break_expr.clone();
                    self.fail(&break_expr, "only a plain `break` is supported");
                    return;
                }
                *i = self.handle_loop_exit(i.clone(), LoopExit::Break);
            }
            Expr::Continue(continue_expr) => {
                if continue_expr.label.is_some() {
                    let continue_expr = continue_expr.clone();
                    self.fail(&continue_expr, "only a plain `continue` is supported");
                    return;
                }
                *i = self.handle_loop_exit(i.clone(), LoopExit::Continue);
            }
            Expr::Binary(binary) => {
                *i = self.handle_binary(binary);
            }
            // Grouping only: the expression inside is what is computed.
            Expr::Paren(paren) => {
                self.visit_expr_mut(&mut paren.expr);
                *i = (*paren.expr).clone();
            }
            Expr::Unary(unary) => {
                *i = self.handle_unary(unary);
            }
            Expr::Field(field) => {
                *i = self.handle_field(field);
            }
            Expr::Index(index) => {
                *i = self.handle_index(index);
            }
            // `()` is not a value a workflow makes; a tuple of values is.
            Expr::Tuple(tuple) if !tuple.elems.is_empty() => {
                *i = self.handle_tuple(tuple);
            }
            Expr::Macro(mac) if mac.mac.path.is_ident("format") => {
                *i = self.handle_format(mac);
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
            // `x op= e` is `x = x op e`.
            Stmt::Expr(Expr::Binary(binary), semi) if compound(&binary.op).is_some() => {
                let op = compound(&binary.op).expect("checked by the guard");
                let (left, right) = (&binary.left, &binary.right);
                let assign: Expr = parse_quote! { #left = #left #op #right };
                *i = Stmt::Expr(assign, *semi);
                self.visit_stmt_mut(i);
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
    /// An operator on values: a built-in the engine evaluates where the branch stands. Rust picks
    /// the operand type, so `a + b` must add two numbers (or strings) of one type.
    fn handle_binary(&mut self, binary: &mut syn::ExprBinary) -> Expr {
        use syn::BinOp;
        let builtin = |name: &str| -> proc_macro2::TokenStream {
            let name = format_ident!("{}", name);
            quote! { ::namu::__macro_exports::ScalarOp::#name }
        };
        let (helper, op) = match binary.op {
            BinOp::Add(_) => ("plus", None),
            BinOp::Sub(_) => ("arith", Some(builtin("Sub"))),
            BinOp::Mul(_) => ("arith", Some(builtin("Mul"))),
            BinOp::Div(_) => ("arith", Some(builtin("Div"))),
            BinOp::Rem(_) => ("arith", Some(builtin("Rem"))),
            BinOp::Eq(_) => ("compare", Some(builtin("Eq"))),
            BinOp::Ne(_) => ("compare", Some(builtin("Ne"))),
            BinOp::Lt(_) => ("compare", Some(builtin("Lt"))),
            BinOp::Le(_) => ("compare", Some(builtin("Le"))),
            BinOp::Gt(_) => ("compare", Some(builtin("Gt"))),
            BinOp::Ge(_) => ("compare", Some(builtin("Ge"))),
            BinOp::And(_) => ("logic", Some(builtin("And"))),
            BinOp::Or(_) => ("logic", Some(builtin("Or"))),
            _ => {
                self.fail(
                    &binary.op,
                    "this operator is not supported in workflows; move it into a task",
                );
                return Expr::Binary(binary.clone());
            }
        };
        self.visit_expr_mut(&mut binary.left);
        self.visit_expr_mut(&mut binary.right);
        self.last_expr_has_value = true;
        let (builder, helper) = (&self.builder_ident, format_ident!("{}", helper));
        let (left, right) = (&binary.left, &binary.right);
        let op = op.map(|op| quote! { #op, });
        parse_quote! { ::namu::__macro_exports::#helper(&#builder, #op #left, #right) }
    }

    /// `value.name`, or `tuple.0`: the closure is the same access in Rust, which rustc checks and
    /// types; the engine finds the field in the value's bytes.
    fn handle_field(&mut self, field: &mut syn::ExprField) -> Expr {
        self.visit_expr_mut(&mut field.base);
        self.last_expr_has_value = true;
        let (builder, base, member) = (&self.builder_ident, &field.base, &field.member);
        let name = match member {
            syn::Member::Named(ident) => ident.to_string(),
            syn::Member::Unnamed(index) => index.index.to_string(),
        };
        parse_quote! {
            ::namu::__macro_exports::field(&#builder, #base, #name, |value| &value.#member)
        }
    }

    /// `list[index]`.
    fn handle_index(&mut self, index: &mut syn::ExprIndex) -> Expr {
        self.visit_expr_mut(&mut index.expr);
        self.visit_expr_mut(&mut index.index);
        self.last_expr_has_value = true;
        let (builder, list, at) = (&self.builder_ident, &index.expr, &index.index);
        parse_quote! { ::namu::__macro_exports::index(&#builder, #list, #at) }
    }

    /// `(a, b, ...)`.
    fn handle_tuple(&mut self, tuple: &mut syn::ExprTuple) -> Expr {
        for elem in tuple.elems.iter_mut() {
            self.visit_expr_mut(elem);
        }
        self.last_expr_has_value = true;
        let (builder, elems) = (&self.builder_ident, tuple.elems.iter());
        parse_quote! { ::namu::__macro_exports::tuple(&#builder, (#(#elems,)*)) }
    }

    /// `format!("...", a, b)`, with `{}` and `{name}` placeholders, each a scalar. Formatting
    /// options (`{:>8}`, `{:.2}`) and positions (`{0}`) are not taken: a task formats that way.
    fn handle_format(&mut self, mac: &mut syn::ExprMacro) -> Expr {
        const USAGE: &str = "a workflow's `format!` takes a string literal with `{}` or `{name}` \
                             placeholders, then one value for each `{}`";
        let parser = syn::punctuated::Punctuated::<Expr, syn::Token![,]>::parse_terminated;
        let Ok(parts) = syn::parse::Parser::parse2(parser, mac.mac.tokens.clone()) else {
            self.fail(&mac, USAGE);
            return Expr::Macro(mac.clone());
        };
        let mut parts = parts.into_iter();
        let Some(Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(template),
            ..
        })) = parts.next()
        else {
            self.fail(&mac, USAGE);
            return Expr::Macro(mac.clone());
        };
        let mut positional = parts;
        let (pieces, holes) = match format_template(&template.value()) {
            Ok(parsed) => parsed,
            Err(message) => {
                self.fail(&template, &message);
                return Expr::Macro(mac.clone());
            }
        };
        let mut args = Vec::new();
        for hole in holes {
            let arg = match hole {
                Some(name) => {
                    let name = format_ident!("{}", name);
                    parse_quote! { #name }
                }
                None => match positional.next() {
                    Some(arg) => arg,
                    None => {
                        self.fail(&template, "more `{}` than values");
                        return Expr::Macro(mac.clone());
                    }
                },
            };
            args.push(arg);
        }
        if let Some(extra) = positional.next() {
            self.fail(&extra, "a value no `{}` writes");
            return Expr::Macro(mac.clone());
        }
        for arg in args.iter_mut() {
            self.visit_expr_mut(arg);
        }
        self.last_expr_has_value = true;
        let builder = &self.builder_ident;
        parse_quote! {
            ::namu::__macro_exports::format(
                &#builder,
                &[#(#pieces),*],
                ::std::vec![#(::namu::__macro_exports::format_arg(#args)),*],
            )
        }
    }

    /// `window(items, N)`: continues once per `N` items of the innermost fan-out, with them.
    /// The size is part of the IR, so it is a literal.
    fn handle_window(&mut self, call: &mut syn::ExprCall) -> Expr {
        const USAGE: &str = "a window is `window(items, N)`, with N an integer literal";
        let size = match call.args.iter().nth(1) {
            Some(Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Int(size),
                ..
            })) if call.args.len() == 2 => size.clone(),
            _ => {
                self.fail(&call, USAGE);
                return Expr::Call(call.clone());
            }
        };
        let mut items = call.args[0].clone();
        self.visit_expr_mut(&mut items);
        self.last_expr_has_value = true;
        let builder = &self.builder_ident;
        parse_quote! { ::namu::__macro_exports::window(&#builder, #items, #size) }
    }

    fn handle_unary(&mut self, unary: &mut syn::ExprUnary) -> Expr {
        let builder = self.builder_ident.clone();
        let negative_number = matches!(
            &*unary.expr,
            Expr::Lit(lit) if matches!(lit.lit, syn::Lit::Int(_) | syn::Lit::Float(_))
        );
        match unary.op {
            // A negative number is a literal, not an expression.
            syn::UnOp::Neg(_) if negative_number => {
                self.last_expr_has_value = true;
                parse_quote! { ::namu::__macro_exports::literal(&#builder, #unary) }
            }
            syn::UnOp::Neg(_) => {
                self.visit_expr_mut(&mut unary.expr);
                self.last_expr_has_value = true;
                let operand = &unary.expr;
                parse_quote! { ::namu::__macro_exports::negate(&#builder, #operand) }
            }
            syn::UnOp::Not(_) => {
                self.visit_expr_mut(&mut unary.expr);
                self.last_expr_has_value = true;
                let operand = &unary.expr;
                parse_quote! { ::namu::__macro_exports::not(&#builder, #operand) }
            }
            _ => {
                self.fail(&unary.op, "this operator is not supported in workflows");
                Expr::Unary(unary.clone())
            }
        }
    }

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

    /// `match`: on an attempt (`match attempt(task(..)) { Ok(..) => .., Err(..) => .. }`), or on
    /// an enum's variants.
    fn handle_match(&mut self, match_expr: &mut ExprMatch) -> Expr {
        if match_expr
            .attrs
            .iter()
            .any(|attr| attr.path().is_ident(ARMS))
        {
            return self.handle_arms(match_expr);
        }
        if attempted(&match_expr.expr).is_some() {
            return self.handle_attempt(match_expr);
        }
        if let Some(arm) = match_expr
            .arms
            .iter()
            .find(|arm| result_arm(&arm.pat).is_some())
        {
            self.fail(
                &arm.pat,
                "a task's failure is matched on through an attempt: \
                 `match attempt(task(..)) { Ok(..) => .., Err(..) => .. }`",
            );
            return Expr::Match(match_expr.clone());
        }
        self.handle_enum_match(match_expr)
    }

    /// `match attempt(task(..)) { Ok(v) => .., Err(e) => .. }`: the call gets an error edge
    /// (design §4.4). `v` is bound to its outputs where it succeeded, `e` to its error where it
    /// failed: the task's own error value, if it fails with a type of its own, else its message.
    /// Several `Err` arms match on the error's variants.
    fn handle_attempt(&mut self, match_expr: &mut ExprMatch) -> Expr {
        const USAGE: &str =
            "only `match attempt(task(..)) { Ok(..) => .., Err(..) => .. }` is supported";
        let (call, task) = attempted(&match_expr.expr).expect("an attempt");
        let (mut call, task) = (call.clone(), task.clone());
        let (mut ok, mut errs) = (None, Vec::new());
        for arm in &match_expr.arms {
            let Some((variant, pat)) = result_arm(&arm.pat) else {
                self.fail(&arm.pat, USAGE);
                return Expr::Match(match_expr.clone());
            };
            if variant == "Err" {
                errs.push(syn::Arm {
                    pat: pat.clone(),
                    ..arm.clone()
                });
            } else if ok.replace((pat.clone(), arm.body.clone())).is_some() {
                self.fail(&arm.pat, "an attempt takes one `Ok` arm");
                return Expr::Match(match_expr.clone());
            }
        }
        let Some((ok_pat, ok_body)) = ok.filter(|_| !errs.is_empty()) else {
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
            let #error = ::namu::__macro_exports::catch::<_, #task>(
                &#builder,
                #then_block,
                #else_block,
            );
        };
        let bind = |pat: &Pat, from: &Ident, body: &Expr| -> Block {
            match pat {
                Pat::Wild(_) => parse_quote!({ #body }),
                _ => parse_quote!({ let #pat = #from; #body }),
            }
        };
        let mut ok_block = bind(&ok_pat, &value, &ok_body);
        let err_block = match &errs[..] {
            [arm] if catch_all(&arm.pat).is_some() => bind(&arm.pat, &error, &arm.body),
            // The error is matched on, as an enum.
            _ => {
                let arms = errs.into_iter();
                parse_quote!({ match #error { #(#arms)* } })
            }
        };
        let mut err_expr = Expr::Block(syn::ExprBlock {
            attrs: Vec::new(),
            label: None,
            block: err_block,
        });
        self.two_way(id, [evaluate, seal], &mut ok_block, Some(&mut err_expr))
    }

    /// `match value { Enum::A => .., Enum::B(x) => .., Enum::C { f, .. } => .., _ => .. }`: a
    /// split per arm, in order, on whether the value is the arm's variant (a built-in), whose
    /// fields the arm binds. rustc checks the arms against the enum, in a match that never runs:
    /// that they are its variants, and cover it.
    fn handle_enum_match(&mut self, match_expr: &mut ExprMatch) -> Expr {
        let id = self.new_control_flow_id();
        let value = format_ident!("__match_{}", id);
        let mut scrutinee = (*match_expr.expr).clone();
        self.visit_expr_mut(&mut scrutinee);
        let checks = match_expr.arms.iter().map(|arm| {
            let pat = &arm.pat;
            quote! { #pat => {} }
        });
        let mut arms = ExprMatch {
            attrs: vec![parse_quote!(#[__namu_arms])],
            expr: parse_quote!(#value),
            ..match_expr.clone()
        };
        let lowered = self.handle_arms(&mut arms);
        // Where rustc says what the arms miss: at the `match`.
        let span = match_expr.match_token.span;
        let check = quote_spanned! {span=>
            #[allow(unused_variables, unreachable_patterns)]
            ::namu::__macro_exports::check_arms(&#value, |__value| match __value {
                #(#checks)*
            });
        };
        parse_quote!({
            let #value = #scrutinee;
            #check
            #lowered
        })
    }

    /// The arms of an enum `match`, on the value `match_expr.expr` names: the first arm's split,
    /// whose other side is the rest of the arms.
    fn handle_arms(&mut self, match_expr: &mut ExprMatch) -> Expr {
        let Expr::Path(value) = &*match_expr.expr else {
            unreachable!("the arms are on a value the macro bound")
        };
        let value = value.path.get_ident().expect("a name").clone();
        let Some((arm, rest)) = match_expr.arms.split_first() else {
            self.fail(&match_expr, "a workflow's `match` needs an arm");
            return Expr::Match(match_expr.clone());
        };
        let body = &arm.body;
        if let Some(binding) = catch_all(&arm.pat) {
            if let Some(next) = rest.first() {
                self.fail(&next.pat, "arms after one that matches anything never run");
                return Expr::Match(match_expr.clone());
            }
            let mut block: Expr = match binding {
                Some(name) => parse_quote!({ let #name = #value; #body }),
                None => parse_quote!({ #body }),
            };
            self.visit_expr_mut(&mut block);
            return block;
        }
        let (variant, bindings) = match variant_arm(&arm.pat) {
            Ok(arm) => arm,
            Err(err) => {
                self.error.get_or_insert(err);
                return Expr::Match(match_expr.clone());
            }
        };
        let builder = self.builder_ident.clone();
        let name = variant.segments.last().expect("a path").ident.to_string();
        let bind = bindings.iter().map(|(field, binding, part)| {
            quote! {
                #[allow(unreachable_patterns)]
                let #binding = ::namu::__variant_part!(
                    #builder,
                    #value,
                    #name,
                    #field,
                    |__value| match __value { #part => __part, _ => unreachable!() }
                );
            }
        });
        // The last arm is what remains, as rustc has checked: the arms cover the enum. A value a
        // task written elsewhere gives may still be another variant, which fails the branch.
        if rest.is_empty() {
            let mut block: Expr = parse_quote!({
                ::namu::__expect_variant!(#builder, #value, #name);
                #(#bind)*
                #body
            });
            self.visit_expr_mut(&mut block);
            return block;
        }
        let mut then: Block = parse_quote!({ #(#bind)* #body });
        let id = self.new_control_flow_id();
        let (then_block, else_block, _) = two_way_blocks(id);
        let evaluate = quote! {
            let __if_condition = ::namu::__macro_exports::is_variant(&#builder, &#value, #name);
        };
        let seal = quote! {
            ::namu::__macro_exports::branch(&#builder, __if_condition, #then_block, #else_block);
        };
        let mut others = Expr::Match(ExprMatch {
            arms: rest.to_vec(),
            ..match_expr.clone()
        });
        self.two_way(id, [evaluate, seal], &mut then, Some(&mut others))
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

    /// `break` or `continue`: an edge out of the innermost loop's body, to its exit or its header.
    /// It records where it leaves from and the loop's variables there, for the phis at its
    /// target. What follows in the same block can never run.
    fn handle_loop_exit(&mut self, expr: Expr, exit: LoopExit) -> Expr {
        let Some((id, vars)) = self.loops.last().cloned() else {
            self.fail(&expr, "`break` and `continue` must be inside a `while`");
            return expr;
        };
        self.last_expr_has_value = false;
        let builder = self.builder_ident.clone();
        let (edges, target) = match exit {
            LoopExit::Break => (
                format_ident!("__while_breaks_{}", id),
                format_ident!("__while_exit_block_{}", id),
            ),
            LoopExit::Continue => (
                format_ident!("__while_continues_{}", id),
                format_ident!("__while_header_block_{}", id),
            ),
        };
        parse_quote! {
            {
                #edges.push((#builder.current_block_id(), vec![#(#vars.id),*]));
                ::namu::__macro_exports::jump(&#builder, #target);
                let __unreachable = #builder.new_block();
                #builder.switch_to_block(__unreachable);
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
                    ::namu::__macro_exports::typed_like(&#builder, __phi_id, &#pre_while_name)
                };
                #name = #phi_val_ident;
                let #phi_node_id = #builder.arena().nodes.len() - 1;
            }
        });

        self.visit_expr_mut(&mut while_expr.cond);
        let cond = &while_expr.cond;

        self.loops.push((id, vars.clone()));
        self.visit_block_mut(&mut while_expr.body);
        self.loops.pop();
        let body_block = &while_expr.body;
        let breaks = format_ident!("__while_breaks_{}", id);
        let continues = format_ident!("__while_continues_{}", id);
        let header_exit = format_ident!("__while_header_exit_{}", id);

        let post_body_captures = vars.iter().map(|name| {
            let post_body_name = format_ident!("__post_body_{}_{}", name, id);
            quote! { let #post_body_name = #name; }
        });

        let body_predecessor_id = format_ident!("__body_predecessor_id_{}", id);

        // The header is entered from before the loop, from the end of the body, and from every
        // `continue`.
        let mut phi_patchers = proc_macro2::TokenStream::new();
        for (index, var) in vars.iter().enumerate() {
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
                    from.extend(#continues.iter().map(|(block, ids)| (*block, ids[#index])));
                }
            });
        }

        // The exit is reached from the header, where the variables are its phis, and from every
        // `break`.
        let exit_phis = vars.iter().enumerate().map(|(index, name)| {
            let phi_val_ident = format_ident!("__{}_phi_val_{}", name, id);
            quote! {
                #name = if #breaks.is_empty() {
                    #phi_val_ident
                } else {
                    let __from = ::std::iter::once((#header_exit, #phi_val_ident.id))
                        .chain(#breaks.iter().map(|(block, ids)| (*block, ids[#index])))
                        .collect();
                    ::namu::__macro_exports::typed_like(&#builder, #builder.phi(__from), &#phi_val_ident)
                };
            }
        });

        parse_quote! {
            {
                #(#pre_while_captures)*
                let mut #breaks: ::std::vec::Vec<(usize, ::std::vec::Vec<usize>)> = ::std::vec::Vec::new();
                let mut #continues: ::std::vec::Vec<(usize, ::std::vec::Vec<usize>)> = ::std::vec::Vec::new();

                let #header_block_id = #builder.new_block();
                let #body_block_id = #builder.new_block();
                let #exit_block_id = #builder.new_block();

                let #parent_predecessor_id = #builder.current_block_id();
                ::namu::__macro_exports::jump(&#builder, #header_block_id);

                #builder.switch_to_block(#header_block_id);
                #(#phi_node_creations)*
                let __while_cond = #cond;
                let #header_exit = #builder.current_block_id();
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

/// The operator of a compound assignment: `+` for `+=`, and so on.
/// A `format!` template's text pieces, and what goes between them: `None` for `{}`, the name for
/// `{name}`. `{{` and `}}` are braces.
fn format_template(template: &str) -> Result<(Vec<String>, Vec<Option<String>>), String> {
    let (mut pieces, mut holes, mut piece) = (Vec::new(), Vec::new(), String::new());
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            ('{', Some('{')) | ('}', Some('}')) => {
                piece.push(c);
                chars.next();
            }
            ('{', _) => {
                let (mut hole, mut closed) = (String::new(), false);
                for c in chars.by_ref() {
                    if c == '}' {
                        closed = true;
                        break;
                    }
                    hole.push(c);
                }
                if !closed {
                    return Err("a `{` with no `}`: write `{{` for a brace".into());
                }
                let hole = hole.trim();
                let named = hole
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphabetic() || c == '_')
                    && hole.chars().all(|c| c.is_alphanumeric() || c == '_');
                if !hole.is_empty() && !named {
                    return Err(format!(
                        "`{{{hole}}}`: a workflow formats with `{{}}` or `{{name}}` only; \
                         a task can format any other way"
                    ));
                }
                pieces.push(std::mem::take(&mut piece));
                holes.push((!hole.is_empty()).then(|| hole.to_string()));
            }
            ('}', _) => return Err("a `}` with no `{`: write `}}` for a brace".into()),
            _ => piece.push(c),
        }
    }
    pieces.push(piece);
    Ok((pieces, holes))
}

fn compound(op: &syn::BinOp) -> Option<syn::BinOp> {
    use syn::BinOp;
    Some(match op {
        BinOp::AddAssign(_) => parse_quote!(+),
        BinOp::SubAssign(_) => parse_quote!(-),
        BinOp::MulAssign(_) => parse_quote!(*),
        BinOp::DivAssign(_) => parse_quote!(/),
        BinOp::RemAssign(_) => parse_quote!(%),
        _ => return None,
    })
}

enum LoopExit {
    Break,
    Continue,
}

/// The blocks of a two-way split: then, else, and where they meet.
fn two_way_blocks(id: usize) -> (Ident, Ident, Ident) {
    (
        format_ident!("__if_then_block_{}", id),
        format_ident!("__if_else_block_{}", id),
        format_ident!("__if_merge_block_{}", id),
    )
}

/// The call in `attempt(call)`, and the path of the task it calls.
fn attempted(expr: &Expr) -> Option<(&Expr, &syn::Path)> {
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
    let Expr::Call(inner) = call else {
        return None;
    };
    let Expr::Path(task) = &*inner.func else {
        return None;
    };
    Some((call, &task.path))
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

/// Marks a `match` the macro made of the arms left after a split, on a value already bound.
const ARMS: &str = "__namu_arms";

/// An arm's pattern that matches anything: `_` (`Some(None)`) or a name for the value (by Rust's
/// naming, a name that starts with a capital is a unit variant instead).
fn catch_all(pat: &Pat) -> Option<Option<&Ident>> {
    match pat {
        Pat::Wild(_) => Some(None),
        Pat::Ident(PatIdent {
            ident,
            subpat: None,
            by_ref: None,
            ..
        }) if !ident.to_string().starts_with(char::is_uppercase) => Some(Some(ident)),
        _ => None,
    }
}

/// A binding of a variant's field: the field (as written in Rust; a tuple variant's by position),
/// the name it is bound to, and a pattern that binds it alone, as `__part`.
type Binding = (String, Ident, proc_macro2::TokenStream);

/// A variant pattern: the variant's path, and what it binds. Its fields are bound to names or
/// left out (`_`, `..`); a pattern inside a field is not supported.
fn variant_arm(pat: &Pat) -> syn::Result<(syn::Path, Vec<Binding>)> {
    const FIELDS: &str = "a variant's fields are bound to names, or left out with `_` or `..`";
    let named = |pat: &Pat| -> syn::Result<Option<Ident>> {
        match pat {
            Pat::Wild(_) => Ok(None),
            Pat::Ident(PatIdent {
                ident,
                subpat: None,
                by_ref: None,
                ..
            }) => Ok(Some(ident.clone())),
            _ => Err(syn::Error::new_spanned(pat, FIELDS)),
        }
    };
    match pat {
        Pat::Path(path) => Ok((path.path.clone(), Vec::new())),
        Pat::Ident(ident) if ident.subpat.is_none() => Ok((ident.ident.clone().into(), Vec::new())),
        Pat::TupleStruct(tuple) => {
            let path = &tuple.path;
            let elems: Vec<&Pat> = tuple.elems.iter().collect();
            let rest = elems.iter().position(|pat| matches!(pat, Pat::Rest(_)));
            if rest.is_some_and(|at| at + 1 != elems.len()) {
                return Err(syn::Error::new_spanned(
                    pat,
                    "`..` goes last in a variant's fields",
                ));
            }
            let fields = &elems[..rest.unwrap_or(elems.len())];
            let mut bindings = Vec::new();
            for (index, field) in fields.iter().enumerate() {
                let Some(name) = named(field)? else {
                    continue;
                };
                let blanks = (0..fields.len()).map(|at| {
                    if at == index {
                        quote!(__part)
                    } else {
                        quote!(_)
                    }
                });
                let part = quote! { #path(#(#blanks,)* ..) };
                bindings.push((index.to_string(), name, part));
            }
            Ok((path.clone(), bindings))
        }
        Pat::Struct(fields) => {
            let path = &fields.path;
            let mut bindings = Vec::new();
            for field in &fields.fields {
                let syn::Member::Named(member) = &field.member else {
                    return Err(syn::Error::new_spanned(field, FIELDS));
                };
                let Some(name) = named(&field.pat)? else {
                    continue;
                };
                let part = quote! { #path { #member: __part, .. } };
                bindings.push((member.to_string(), name, part));
            }
            Ok((path.clone(), bindings))
        }
        _ => Err(syn::Error::new_spanned(
            pat,
            "an arm of a workflow's `match` is a variant (`E::A`, `E::B(x)`, `E::C { f, .. }`), \
             `_`, or a name",
        )),
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
    let inputs = match workflow_inputs(&func, &builder_ident) {
        Ok(inputs) => inputs,
        Err(err) => return err.to_compile_error().into(),
    };
    let mut visitor = WorkflowVisitor::new(builder_ident.clone());
    visitor.enter_scope();
    // A `mut` parameter is a variable loops carry, as a `let mut` is.
    for input in inputs.iter().filter(|input| input.mutable) {
        visitor.insert_var(input.name.clone());
    }
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

    let input_lets = inputs.iter().map(|input| &input.binding);

    let expanded = quote! {
        #[allow(unused_assignments)]
        #[allow(unused_braces)]
        #[allow(unused_mut)]
        pub fn #func_name() -> ::namu::__macro_exports::Graph<#return_type> {
            let #builder_ident = ::namu::__macro_exports::Builder::<#return_type>::new();
            #(#input_lets)*

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

/// A workflow parameter, which is one of its inputs.
struct WorkflowInput {
    name: Ident,
    mutable: bool,
    /// The `let` that binds the name to the input's value.
    binding: proc_macro2::TokenStream,
}

/// The workflow's parameters. A parameter is a plain name with a type.
fn workflow_inputs(func: &ItemFn, builder: &Ident) -> syn::Result<Vec<WorkflowInput>> {
    func.sig
        .inputs
        .iter()
        .map(|arg| {
            let FnArg::Typed(arg) = arg else {
                return Err(syn::Error::new_spanned(arg, "a workflow takes no `self`"));
            };
            let Pat::Ident(PatIdent {
                ident,
                mutability,
                by_ref: None,
                subpat: None,
                ..
            }) = &*arg.pat
            else {
                return Err(syn::Error::new_spanned(
                    &arg.pat,
                    "a workflow parameter is a plain name with a type: `name: Type`",
                ));
            };
            let ty = &arg.ty;
            let name = ident.to_string();
            let binding = quote! {
                let #mutability #ident = ::namu::__macro_exports::input::<#ty, _>(&#builder, #name);
            };
            Ok(WorkflowInput {
                name: ident.clone(),
                mutable: mutability.is_some(),
                binding,
            })
        })
        .collect()
}
