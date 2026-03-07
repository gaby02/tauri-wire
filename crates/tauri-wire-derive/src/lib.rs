use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, Data, Fields, Type, PathSegment};

/// Derive the `WireMessage` trait for a struct with named fields.
///
/// # Requirements
///
/// - The struct must have named fields only (no tuple or unit structs).
/// - All fields must be one of: `u8`, `u16`, `u32`, `u64`, `i8`, `i16`, `i32`, `i64`,
///   `f32`, `f64`, `bool`.
/// - The struct must have `#[wire(msg_type = N)]` where N is a `u8` literal.
///
/// # Example
///
/// ```rust,ignore
/// use tauri_wire::WireMessage;
///
/// #[derive(WireMessage)]
/// #[wire(msg_type = 1)]
/// struct Tick {
///     ts_ms: i64,
///     price: f64,
///     volume: f64,
///     side: u8,
/// }
/// ```
#[proc_macro_derive(WireMessage, attributes(wire))]
pub fn derive_wire_message(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match impl_wire_message(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn impl_wire_message(input: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let name = &input.ident;
    let msg_type = extract_msg_type(&input.attrs)?;

    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(named) => &named.named,
            _ => return Err(syn::Error::new_spanned(
                name,
                "WireMessage can only be derived for structs with named fields",
            )),
        },
        _ => return Err(syn::Error::new_spanned(
            name,
            "WireMessage can only be derived for structs",
        )),
    };

    let mut encode_stmts = Vec::new();
    let mut decode_stmts = Vec::new();
    let mut field_names = Vec::new();
    let mut total_size: usize = 0;

    for field in fields {
        let field_name = field.ident.as_ref().unwrap();
        let ty = &field.ty;
        let info = type_info(ty)?;
        let size = info.size;

        encode_stmts.push(info.encode_expr(field_name));
        decode_stmts.push(info.decode_expr(field_name, total_size));
        field_names.push(field_name.clone());
        total_size += size;
    }

    let total_size_lit = total_size;

    Ok(quote! {
        impl ::tauri_wire::WireMessage for #name {
            const MSG_TYPE: u8 = #msg_type;

            fn encode(&self, buf: &mut Vec<u8>) {
                #(#encode_stmts)*
            }

            fn encoded_size(&self) -> usize {
                #total_size_lit
            }

            fn decode(data: &[u8]) -> Option<(Self, usize)> {
                if data.len() < #total_size_lit {
                    return None;
                }
                #(#decode_stmts)*
                Some((Self { #(#field_names),* }, #total_size_lit))
            }
        }
    })
}

fn extract_msg_type(attrs: &[syn::Attribute]) -> syn::Result<u8> {
    for attr in attrs {
        if attr.path().is_ident("wire") {
            let mut msg_type = None;
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("msg_type") {
                    let value = meta.value()?;
                    let lit: syn::LitInt = value.parse()?;
                    msg_type = Some(lit.base10_parse::<u8>()?);
                    Ok(())
                } else {
                    Err(meta.error("expected `msg_type`"))
                }
            })?;
            if let Some(v) = msg_type {
                return Ok(v);
            }
        }
    }
    Err(syn::Error::new(
        proc_macro2::Span::call_site(),
        "missing #[wire(msg_type = N)] attribute",
    ))
}

struct TypeInfo {
    size: usize,
    kind: TypeKind,
}

enum TypeKind {
    U8,
    Bool,
    LeNum { rust_type: proc_macro2::TokenStream, size: usize },
}

impl TypeInfo {
    fn encode_expr(&self, field: &syn::Ident) -> proc_macro2::TokenStream {
        match &self.kind {
            TypeKind::U8 => quote! { buf.push(self.#field); },
            TypeKind::Bool => quote! { buf.push(if self.#field { 1 } else { 0 }); },
            TypeKind::LeNum { .. } => quote! {
                buf.extend_from_slice(&self.#field.to_le_bytes());
            },
        }
    }

    fn decode_expr(&self, field: &syn::Ident, offset: usize) -> proc_macro2::TokenStream {
        match &self.kind {
            TypeKind::U8 => quote! {
                let #field = data[#offset];
            },
            TypeKind::Bool => quote! {
                let #field = data[#offset] != 0;
            },
            TypeKind::LeNum { rust_type, size } => {
                let end = offset + size;
                quote! {
                    let #field = #rust_type::from_le_bytes(
                        data[#offset..#end].try_into().ok()?
                    );
                }
            }
        }
    }
}

fn type_info(ty: &Type) -> syn::Result<TypeInfo> {
    let path = match ty {
        Type::Path(tp) => &tp.path,
        _ => return Err(syn::Error::new_spanned(ty, "unsupported field type")),
    };

    let segment: &PathSegment = path.segments.last()
        .ok_or_else(|| syn::Error::new_spanned(ty, "unsupported field type"))?;

    let ident = segment.ident.to_string();
    match ident.as_str() {
        "u8" => Ok(TypeInfo { size: 1, kind: TypeKind::U8 }),
        "bool" => Ok(TypeInfo { size: 1, kind: TypeKind::Bool }),
        "u16" => Ok(TypeInfo { size: 2, kind: TypeKind::LeNum { rust_type: quote!(u16), size: 2 } }),
        "i8" => Ok(TypeInfo { size: 1, kind: TypeKind::LeNum { rust_type: quote!(i8), size: 1 } }),
        "i16" => Ok(TypeInfo { size: 2, kind: TypeKind::LeNum { rust_type: quote!(i16), size: 2 } }),
        "u32" => Ok(TypeInfo { size: 4, kind: TypeKind::LeNum { rust_type: quote!(u32), size: 4 } }),
        "i32" => Ok(TypeInfo { size: 4, kind: TypeKind::LeNum { rust_type: quote!(i32), size: 4 } }),
        "u64" => Ok(TypeInfo { size: 8, kind: TypeKind::LeNum { rust_type: quote!(u64), size: 8 } }),
        "i64" => Ok(TypeInfo { size: 8, kind: TypeKind::LeNum { rust_type: quote!(i64), size: 8 } }),
        "f32" => Ok(TypeInfo { size: 4, kind: TypeKind::LeNum { rust_type: quote!(f32), size: 4 } }),
        "f64" => Ok(TypeInfo { size: 8, kind: TypeKind::LeNum { rust_type: quote!(f64), size: 8 } }),
        _ => Err(syn::Error::new_spanned(
            ty,
            format!("unsupported field type `{ident}`. WireMessage derive supports: u8, u16, u32, u64, i8, i16, i32, i64, f32, f64, bool"),
        )),
    }
}
