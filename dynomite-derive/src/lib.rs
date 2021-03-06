//! Dynomite-derive provides procedural macros for deriving dynomite types
//! for your structs
//!
//! # Examples
//!
//! ```ignore
//! use dynomite::{Item, FromAttributes, Attributes};
//! use dynomite::dynamodb::AttributeValue;
//!
//! // derive Item
//! #[derive(Item, PartialEq, Debug, Clone)]
//! struct Person {
//!   #[hash] id: String
//! }
//!
//! fn main() {
//!   let person = Person { id: "123".into() };
//!   // convert person to string keys and attribute values
//!   let attributes: Attributes = person.clone().into();
//!   // convert attributes into person type
//!   assert_eq!(person, Person::from_attrs(attributes).unwrap());
//!
//!   // dynamodb types require only primary key attributes and may contain
//!   // other fields. when looking up items only those key attributes are required
//!   // dynomite derives a new {Name}Key struct for your which contains
//!   // only those and also implements Item
//!   let key = PersonKey { id: "123".into() };
//!   let key_attributes: Attributes = key.clone().into();
//!   // convert attributes into person type
//!   assert_eq!(key, PersonKey::from_attrs(key_attributes).unwrap());
//! }
//! ```

extern crate proc_macro;

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{quote, ToTokens};
use syn::{
    Data::{Enum, Struct},
    DataStruct, DeriveInput, Field, Fields, Ident, Meta, Variant, Visibility,
};

/// Derives `dynomite::Item` type for struts with named fields
///
/// # Attributes
///
/// * `#[hash]` - required attribute, expected to be applied the target [hash attribute](https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/HowItWorks.CoreComponents.html#HowItWorks.CoreComponents.PrimaryKey) field with an derivable DynamoDB attribute value of String, Number or Binary
/// * `#[range]` - optional attribute, may be applied to one target [range attribute](https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/HowItWorks.CoreComponents.html#HowItWorks.CoreComponents.SecondaryIndexes) field with an derivable DynamoDB attribute value of String, Number or Binary
///
/// # Panics
///
/// This proc macro will panic when applied to other types
#[proc_macro_derive(Item, attributes(hash, range))]
pub fn derive_item(input: TokenStream) -> TokenStream {
    let ast = syn::parse_macro_input!(input);
    let gen = expand_item(ast);
    gen.into_token_stream().into()
}

/// Derives `dynomite::Attribute` for enum types
///
/// # Panics
///
/// This proc macro will panic when applied to other types
#[proc_macro_derive(Attribute)]
pub fn derive_attribute(input: TokenStream) -> TokenStream {
    let ast = syn::parse_macro_input!(input);
    let gen = expand_attribute(ast);
    gen.into_token_stream().into()
}

fn expand_attribute(ast: DeriveInput) -> impl ToTokens {
    let name = &ast.ident;
    match ast.data {
        Enum(variants) => {
            make_dynomite_attr(name, &variants.variants.into_iter().collect::<Vec<_>>())
        }
        _ => panic!("Dynomite Attributes can only be generated for enum types"),
    }
}

/// impl ::dynomite::Attribute for Name {
///   fn into_attr(self) -> ::dynomite::dynamodb::AttributeValue {
///     let arm = match self {
///        Name::Variant => "Variant".to_string()
///     };
///     ::dynomite::dynamodb::AttributeValue {
///        s: Some(arm),
///        ..Default::default()
///     }
///   }
///   fn from_attr(value: ::dynomite::dynamodb::AttributeValue) -> Result<Self, ::dynomite::AttributeError> {
///     value.s.ok_or(::dynomite::AttributeError::InvalidType)
///       .and_then(|value| match &value[..] {
///          "Variant" => Ok(Name::Variant),
///          _ => Err(::dynomite::AttributeError::InvalidFormat)
///       })
///   }
/// }
fn make_dynomite_attr(
    name: &Ident,
    variants: &[Variant],
) -> impl ToTokens {
    let attr = quote!(::dynomite::Attribute);
    let err = quote!(::dynomite::AttributeError);
    let into_match_arms = variants.iter().map(|var| {
        let vname = &var.ident;
        quote! {
            #name::#vname => stringify!(#vname).to_string(),
        }
    });
    let from_match_arms = variants.iter().map(|var| {
        let vname = &var.ident;
        quote! {
            stringify!(#vname) => Ok(#name::#vname),
        }
    });

    quote! {
        impl #attr for #name {
            fn into_attr(self) -> ::dynomite::dynamodb::AttributeValue {
                let arm = match self {
                    #(#into_match_arms)*
                };
                ::dynomite::dynamodb::AttributeValue {
                    s: Some(arm),
                    ..Default::default()
                }
            }
            fn from_attr(value: ::dynomite::dynamodb::AttributeValue) -> Result<Self, #err> {
                value.s.ok_or(::dynomite::AttributeError::InvalidType)
                    .and_then(|value| match &value[..] {
                        #(#from_match_arms)*
                        _ => Err(::dynomite::AttributeError::InvalidFormat)
                    })
            }
        }
    }
}

fn expand_item(ast: DeriveInput) -> impl ToTokens {
    let name = &ast.ident;
    let vis = &ast.vis;
    match ast.data {
        Struct(DataStruct { fields, .. }) => match fields {
            Fields::Named(named) => {
                make_dynomite_item(vis, name, &named.named.into_iter().collect::<Vec<_>>())
            }
            _ => panic!("Dynomite Items require named fields"),
        },
        _ => panic!("Dynomite Items can only be generated for structs"),
    }
}

fn make_dynomite_item(
    vis: &Visibility,
    name: &Ident,
    fields: &[Field],
) -> impl ToTokens {
    let dynamodb_traits = get_dynomite_item_traits(vis, name, fields);
    let from_attribute_map = get_from_attributes_trait(name, fields);
    let to_attribute_map = get_to_attribute_map_trait(name, fields);

    quote! {
        #from_attribute_map
        #to_attribute_map
        #dynamodb_traits
    }
}

fn get_to_attribute_map_trait(
    name: &Ident,
    fields: &[Field],
) -> impl ToTokens {
    let attributes = quote!(::dynomite::Attributes);
    let from = quote!(::std::convert::From);
    let to_attribute_map = get_to_attribute_map_function(name, fields);

    quote! {
        impl #from<#name> for #attributes {
            #to_attribute_map
        }
    }
}

fn get_to_attribute_map_function(
    name: &Ident,
    fields: &[Field],
) -> impl ToTokens {
    let to_attribute_value = quote!(::dynomite::Attribute::into_attr);

    let field_conversions = fields.iter().map(|field| {
        let field_name = &field.ident;
        quote! {
            values.insert(
                stringify!(#field_name).to_string(),
                #to_attribute_value(item.#field_name)
            );
        }
    });

    quote! {
        fn from(item: #name) -> Self {
            let mut values = Self::new();
            #(#field_conversions)*
            values
        }
    }
}

///
/// impl ::dynomite::FromAttributes for Name {
///   fn from_attrs(mut item: ::dynomite::Attributes) -> Result<Self, ::dynomite::Error> {
///     Ok(Self {
///        field_name: ::dynomite::Attribute::from_attr(
///           item.remove("field_name").ok_or(Error::MissingField { name: "field_name".into() })?
///        )
///      })
///   }
/// }
fn get_from_attributes_trait(
    name: &Ident,
    fields: &[Field],
) -> impl ToTokens {
    let from_attrs = quote!(::dynomite::FromAttributes);
    let from_attribute_map = get_from_attributes_function(fields);

    quote! {
        impl #from_attrs for #name {
            #from_attribute_map
        }
    }
}

fn get_from_attributes_function(fields: &[Field]) -> impl ToTokens {
    let attributes = quote!(::dynomite::Attributes);
    let from_attribute_value = quote!(::dynomite::Attribute::from_attr);
    let err = quote!(::dynomite::AttributeError);
    let field_conversions = fields.iter().map(|field| {
        let field_name = &field.ident;
        quote! {
            #field_name: #from_attribute_value(
                attrs.remove(stringify!(#field_name))
                    .ok_or(::dynomite::AttributeError::MissingField { name: stringify!(#field_name).to_string() })?
            )?
        }
    });

    quote! {
        fn from_attrs(mut attrs: #attributes) -> Result<Self, #err> {
            Ok(Self {
                #(#field_conversions),*
            })
        }
    }
}

fn get_dynomite_item_traits(
    vis: &Visibility,
    name: &Ident,
    fields: &[Field],
) -> impl ToTokens {
    let impls = get_item_impls(vis, name, fields);

    quote! {
        #impls
    }
}

fn get_item_impls(
    vis: &Visibility,
    name: &Ident,
    fields: &[Field],
) -> impl ToTokens {
    let item_trait = get_item_trait(name, fields);
    let key_struct = get_key_struct(vis, name, fields);

    quote! {
        #item_trait
        #key_struct
    }
}

///
/// impl ::dynomite::Item for Name {
///   fn key(&self) -> ::std::collections::HashMap<String, ::dynomite::dynamodb::AttributeValue> {
///     let mut keys = ::std::collections::HashMap::new();
///     keys.insert("field_name", to_attribute_value(field));
///     keys
///   }
/// }
///
fn get_item_trait(
    name: &Ident,
    fields: &[Field],
) -> impl ToTokens {
    let item = quote!(::dynomite::Item);
    let attribute_map = quote!(
        ::std::collections::HashMap<String, ::dynomite::dynamodb::AttributeValue>
    );
    let hash_key_name = field_name_with_attribute(&fields, "hash");
    let range_key_name = field_name_with_attribute(&fields, "range");

    let hash_key_insert = get_key_inserter(&hash_key_name);
    let range_key_insert = get_key_inserter(&range_key_name);

    hash_key_name
        .map(|_| {
            quote! {
                impl #item for #name {
                    fn key(&self) -> #attribute_map {
                        let mut keys = ::std::collections::HashMap::new();
                        #hash_key_insert
                        #range_key_insert
                        keys
                    }
                }
            }
        })
        .unwrap_or(quote! {})
}

fn field_name_with_attribute(
    fields: &[Field],
    attribute_name: &str,
) -> Option<Ident> {
    field_with_attribute(fields, attribute_name).map(|field| {
        field.ident.unwrap_or_else(|| {
            panic!(
                "should have an identifier with an {} attribute",
                attribute_name
            )
        })
    })
}

fn field_with_attribute(
    fields: &[Field],
    attribute_name: &str,
) -> Option<Field> {
    let mut fields = fields.iter().cloned().filter(|field| {
        field.attrs.iter().any(|attr| match attr.parse_meta() {
            Ok(Meta::Word(name)) => name == attribute_name,
            _ => false,
        })
    });
    let field = fields.next();
    if fields.next().is_some() {
        panic!("Can't set more than one {} key", attribute_name);
    }
    field
}

/// keys.insert(
///   "field_name", to_attribute_value(field)
/// )
fn get_key_inserter(field_name: &Option<Ident>) -> impl ToTokens {
    let to_attribute_value = quote!(::dynomite::Attribute::into_attr);
    field_name
        .as_ref()
        .map(|field_name| {
            quote! {
                keys.insert(
                    stringify!(#field_name).to_string(),
                    #to_attribute_value(self.#field_name.clone())
                );
            }
        })
        .unwrap_or(quote!())
}

/// #[derive](Item, Debug, Clone, PartialEq)
/// pub struct NameKey {
///    hash_key,
///    range_key
/// }
fn get_key_struct(
    vis: &Visibility,
    name: &Ident,
    fields: &[Field],
) -> impl ToTokens {
    // fixme: this `Span` ref is the only dependency we have on the proc_macro2 crate
    // is this really needed?
    let name = Ident::new(&format!("{}Key", name), Span::call_site());

    let hash_key = field_with_attribute(&fields, "hash");
    let range_key = field_with_attribute(&fields, "range")
        .map(|mut range_key| {
            range_key.attrs = vec![];
            quote! {#range_key}
        })
        .unwrap_or(quote!());

    hash_key
        .map(|mut hash_key| {
            hash_key.attrs = vec![];
            quote! {
                #[derive(Item, Debug, Clone, PartialEq)]
                #vis struct #name {
                    #hash_key,
                    #range_key
                }
            }
        })
        .unwrap_or(quote!())
}
