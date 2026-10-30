#![warn(clippy::all)]

mod rcize;

use proc_macro::TokenStream;

/// Wraps save-struct fields in `RcCell` / `RcRef` so the original binary layout stays intact.
#[proc_macro_attribute]
pub fn rcize_fields(_: TokenStream, input: TokenStream) -> TokenStream {
    rcize::rcize_fields(input)
}

// These derives used to generate the web UI. The terminal editor does not use them.
// They stay so the save structs can keep the attributes without pulling in Yew.

#[proc_macro_derive(RawUi)]
pub fn raw_ui_derive(_input: TokenStream) -> TokenStream {
    TokenStream::new()
}

#[proc_macro_derive(RawUiRoot)]
pub fn raw_ui_derive_root(_input: TokenStream) -> TokenStream {
    TokenStream::new()
}

#[proc_macro_derive(RawUiChildren)]
pub fn raw_ui_children_derive(_input: TokenStream) -> TokenStream {
    TokenStream::new()
}
