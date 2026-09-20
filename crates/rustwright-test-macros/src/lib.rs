//! Procedural macros for the Rustwright test runner.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{parse_macro_input, ItemFn, Visibility};

/// Mark an `async fn` as a Rustwright browser test.
///
/// The function must be async, take a [`TestContext`] by value and return
/// `rustwright_test::Result<()>`. The macro generates a `#[test]` entry point
/// that launches the browser, provides an isolated context and page, and closes
/// everything when the test finishes.
///
/// ```ignore
/// #[rustwright_test]
/// async fn loads_example(context: TestContext) -> rustwright_test::Result<()> {
///     context.page.goto("example.com").await?;
///     Ok(())
/// }
/// ```
///
/// [`TestContext`]: https://docs.rs/rustwright-test
#[proc_macro_attribute]
pub fn rustwright_test(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let function = parse_macro_input!(item as ItemFn);

    if function.sig.asyncness.is_none() {
        return syn::Error::new_spanned(
            function.sig.fn_token,
            "rustwright_test functions must be `async`",
        )
        .to_compile_error()
        .into();
    }

    let name = function.sig.ident.clone();
    let visibility = function.vis.clone();
    let implementation_name = format_ident!("__rustwright_test_impl_{}", name);

    let mut implementation = function.clone();
    implementation.sig.ident = implementation_name.clone();
    implementation.vis = Visibility::Inherited;

    let expanded = quote! {
        #implementation

        #[test]
        #visibility fn #name() {
            ::rustwright_test::run_test(stringify!(#name), |context| {
                #implementation_name(context)
            });
        }
    };

    expanded.into()
}
