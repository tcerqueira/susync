extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, punctuated::Punctuated, Expr, ExprCall, ExprMethodCall, Pat, PatIdent, PatType};

#[proc_macro]
pub fn suspend(input: TokenStream) -> TokenStream {
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
    // Leave normal args the same and modify closure argument
    let found_first_closure = false;
    let gen_args = args
        .iter()
        .rev()
        .map(|arg| match arg {
            Expr::Closure(expr_closure) if !found_first_closure => Expr::Closure(generate_closure(expr_closure)),
            _ => arg.clone(),
        })
        .collect::<Vec<_>>();

    match input {
        Expr::Call(expr_call) => {
            let call = ExprCall {
                args: Punctuated::from_iter(gen_args.into_iter().rev()),
                ..expr_call
            };
            quote! {
                ::susync::suspend(|handle| {
                    #call
                })
            }.into()
        },
        Expr::MethodCall(method_call) => {
            let call = ExprMethodCall {
                args: Punctuated::from_iter(gen_args.into_iter().rev()),
                ..method_call
            };
            quote! {
                ::susync::suspend(|handle| {
                    #call
                })
            }.into()
        },
        _ => panic!("suspend macro can only be applied to function/method calls"),
    }
}

// Helper function to generate the closure
fn generate_closure(closure: &syn::ExprClosure) -> syn::ExprClosure {
    let args = closure.inputs
        .iter()
        .flat_map(|arg_pat| match arg_pat {
            Pat::Ident(PatIdent{ident, ..}) => Some(ident),
            Pat::Type(PatType{pat, ..}) => match pat.as_ref() {
                Pat::Ident(PatIdent{ident, ..}) => Some(ident),
                _ => None,
            },
            Pat::Wild(_) => None,
            _ => panic!("invalid closure arguments"),
        })
        .collect::<Vec<_>>();

    let body = &closure.body;
    let body = quote! {
        {
            handle.complete((#(#args)*));
            #body
        }
    };

    syn::ExprClosure {
        body: Box::new(syn::Expr::Verbatim(body)),
        ..closure.clone()
    }
}
