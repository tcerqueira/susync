//! Support crate that contains the function-like procedural macros for [susync].
//!
//! All documentation lives in that crate.
//!
//! [susync]: https://docs.rs/susync

extern crate proc_macro;

use proc_macro2::{Span, TokenStream};
use quote::{quote, quote_spanned};
use syn::{
    parse_macro_input, punctuated::Punctuated, Expr, ExprCall, ExprMethodCall, Pat, PatIdent,
    PatType,
};

/// Generate the boilerplate for the use case where the future output is equal, or similar, to the callback arguments.
///
/// See full [documentation] for more details.
///
/// [documentation]: https://docs.rs/susync
#[proc_macro]
pub fn sus(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as Expr);
    // Extract method/function call arguments
    let args = match &input {
        Expr::Call(expr_call) => &expr_call.args,
        Expr::MethodCall(method_call) => &method_call.args,
        _ => panic!("suspend macro can only be applied to function/method calls"),
    };
    // Extract the last closure argument from the function call
    let _ = args
        .iter()
        .rev()
        .find(|arg| matches!(arg, syn::Expr::Closure(_)))
        .expect("expected closure as an argument");

    let handle_ident = quote_spanned!(Span::mixed_site()=> handle);
    // Leave normal args the same and modify closure argument
    let mut found_first_closure = false;
    let gen_args = args
        .into_iter()
        .rev()
        .map(|arg| match arg {
            Expr::Closure(expr_closure) if !found_first_closure => {
                found_first_closure = true;
                Expr::Closure(generate_closure(&handle_ident, expr_closure.clone()))
            }
            _ => arg.clone(),
        })
        .collect::<Vec<_>>();

    let call_fn = match input {
        Expr::Call(expr_call) => {
            let call = ExprCall {
                args: Punctuated::from_iter(gen_args.into_iter().rev()),
                ..expr_call
            };
            quote! {
                #call;
            }
        }
        Expr::MethodCall(method_call) => {
            let call = ExprMethodCall {
                args: Punctuated::from_iter(gen_args.into_iter().rev()),
                ..method_call
            };
            quote! {
                #call;
            }
        }
        _ => panic!("suspend macro can only be applied to function/method calls"),
    };

    quote! {
        ::susync::suspend(|#handle_ident| {
            let _ = #call_fn;
        })
    }
    .into()
}

// Helper function to generate the closure
fn generate_closure(captured_handle: &TokenStream, closure: syn::ExprClosure) -> syn::ExprClosure {
    let args = closure
        .inputs
        .iter()
        .flat_map(|arg_pat| match arg_pat {
            Pat::Ident(PatIdent { ident, .. }) => Some(ident),
            Pat::Type(PatType { pat, .. }) => match pat.as_ref() {
                Pat::Ident(PatIdent { ident, .. }) => Some(ident),
                _ => None,
            },
            Pat::Wild(_) => None,
            _ => panic!("invalid closure arguments"),
        })
        .collect::<Vec<_>>();

    let handle_stmt = if args.len() == 1 {
        quote! {
            #captured_handle.complete(#(#args.to_owned())*)
        }
    } else {
        quote! {
            #captured_handle.complete(( #(#args.to_owned(),)* ))
        }
    };

    let body = &closure.body;
    let body = quote! {
        {
            let expr_result = #body;
            // #(let #args = ::std::borrow::ToOwned::to_owned(#args);)*
            #handle_stmt;
            expr_result
        }
    };

    syn::ExprClosure {
        body: Box::new(syn::Expr::Verbatim(body)),
        ..closure
    }
}
