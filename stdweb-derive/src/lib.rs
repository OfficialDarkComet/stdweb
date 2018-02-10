#![recursion_limit="512"]

extern crate proc_macro;
extern crate syn;
#[macro_use]
extern crate quote;

use proc_macro::TokenStream;
use syn::DeriveInput;

fn get_meta_items( attr: &syn::Attribute ) -> Option< Vec< syn::NestedMeta > > {
    if attr.path.segments.len() == 1 && attr.path.segments[0].ident == "reference" {
        match attr.interpret_meta() {
            Some( syn::Meta::List( ref meta ) ) => Some( meta.nested.iter().cloned().collect() ),
            _ => {
                panic!( "Unrecognized meta item type!" );
            }
        }
    } else {
        None
    }
}

#[proc_macro_derive(ReferenceType, attributes(reference))]
pub fn derive_reference( input: TokenStream ) -> TokenStream {
    let input: DeriveInput = syn::parse( input ).unwrap();

    let name = input.ident;
    let generics_params = &input.generics.params;

    let mut instance_of = None;
    let mut subclass_of = Vec::new();

    for meta_items in input.attrs.iter().filter_map( get_meta_items ) {
        for meta in meta_items {
            match meta {
                syn::NestedMeta::Meta( syn::Meta::NameValue( ref meta ) ) if meta.ident == "instance_of" => {
                    if instance_of.is_some() {
                        panic!( "Duplicate '#[reference(instance_of)]'!" );
                    }

                    if let syn::Lit::Str( ref str ) = meta.lit {
                        instance_of = Some( str.value() );
                    } else {
                        panic!( "The value of '#[reference(instance_of = ...)]' is not a string!" );
                    }
                },
                syn::NestedMeta::Meta( syn::Meta::List( ref meta ) ) if meta.ident == "subclass_of" => {
                    for nested in &meta.nested {
                        match *nested {
                            syn::NestedMeta::Meta( ref nested ) => {
                                match *nested {
                                    syn::Meta::Word( ident ) => {
                                        subclass_of.push( ident );
                                    },
                                    _ => panic!( "The value of '#[reference(subclass_of(...))]' is invalid!" )
                                }
                            },
                            _ => panic!( "The value of '#[reference(subclass_of(...))]' is invalid!" )
                        }
                    }
                },
                syn::NestedMeta::Meta( meta ) => {
                    panic!( "Unrecognized attribute: '#[reference({})]'", meta.name() );
                },
                _ => panic!( "Unrecognized attribute!" )
            }
        }
    }

    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    match input.data {
        syn::Data::Struct( ref data ) => {
            match data.fields {
                syn::Fields::Unnamed( ref fields ) => {
                    fn invalid_structure() {
                        panic!( "The structure can only have either (Reference) or (Reference, PhantomData)!" );
                    }

                    let fields = &fields.unnamed;
                    if fields.len() != 1 && fields.len() != 2 {
                        invalid_structure();
                    }

                    let mut fields_iter = fields.iter();
                    match fields_iter.next().unwrap().ty {
                        syn::Type::Path( ref ty_path ) => {
                            if ty_path.qself.is_some() {
                                invalid_structure();
                            }

                            let segs: Vec< _ > = ty_path.path.segments.iter().collect();
                            if segs.last().unwrap().ident != "Reference" || !segs.last().unwrap().arguments.is_empty() {
                                invalid_structure();
                            }

                            match segs.len() {
                                1 => {},
                                2 => {
                                    if segs.first().unwrap().ident != "stdweb" {
                                        invalid_structure();
                                    }
                                },
                                _ => invalid_structure()
                            }
                        },
                        _ => invalid_structure()
                    }

                    match fields_iter.next().map( |field| &field.ty ) {
                        Some( &syn::Type::Path( ref ty_path ) ) => {
                            if ty_path.qself.is_some() {
                                invalid_structure();
                            }

                            let segs: Vec< _ > = ty_path.path.segments.iter().collect();
                            if segs.last().unwrap().ident != "PhantomData" {
                                invalid_structure();
                            }
                        },
                        Some( _ ) => invalid_structure(),
                        None => {}
                    }
                },
                _ => panic!( "Only tuple structures are supported!" )
            }
        },
        _ => panic!( "Only tuple structures are supported!" )
    }

    let mut default_args = Vec::new();
    for param in input.generics.params.iter() {
        match *param {
            syn::GenericParam::Type( _ ) => {
                default_args.push( quote! {
                    ::std::default::Default::default()
                });
            },
            _ => {}
        }
    }
    let default_args = quote! { #(#default_args),* };
    let instance_of_impl = match instance_of {
        Some( js_name ) => {
            quote! {
                impl #impl_generics ::stdweb::InstanceOf for #name #ty_generics {
                    #[inline]
                    fn instance_of( reference: &::stdweb::Reference ) -> bool {
                        __js_raw_asm!(
                            concat!( "return (Module.STDWEB_PRIVATE.acquire_js_reference( $0 ) instanceof ", #js_name, ") | 0;" ),
                            reference.as_raw()
                        ) == 1
                    }
                }
            }
        },
        None => quote! {}
    };

    let subclass_of_impl: Vec< _ > = subclass_of.into_iter().map( |target| {
        let target: syn::Ident = target.into();
        quote! {
            impl #impl_generics From< #name #ty_generics > for #target #where_clause {
                #[inline]
                fn from( value: #name #ty_generics ) -> Self {
                    let reference: ::stdweb::Reference = value.into();
                    unsafe {
                        <#target as ::stdweb::ReferenceType>::from_reference_unchecked( reference )
                    }
                }
            }

            impl #impl_generics ::stdweb::unstable::TryFrom< #target > for #name #ty_generics #where_clause {
                type Error = ::stdweb::private::ConversionError;

                #[inline]
                fn try_from( value: #target ) -> Result< Self, Self::Error > {
                    use ::stdweb::unstable::TryInto;
                    let reference: ::stdweb::Reference = value.into();
                    reference.try_into()
                }
            }
        }
    }).collect();

    let expanded = quote! {
        #(#subclass_of_impl)*
        #instance_of_impl

        impl #impl_generics AsRef< ::stdweb::Reference > for #name #ty_generics #where_clause {
            #[inline]
            fn as_ref( &self ) -> &::stdweb::Reference {
                &self.0
            }
        }

        impl #impl_generics ::stdweb::ReferenceType for #name #ty_generics #where_clause {
            #[inline]
            unsafe fn from_reference_unchecked( reference: ::stdweb::Reference ) -> Self {
                #name( reference, #default_args )
            }
        }

        impl #impl_generics From< #name #ty_generics > for ::stdweb::Reference #where_clause {
            #[inline]
            fn from( value: #name #ty_generics ) -> Self {
                value.0
            }
        }

        impl #impl_generics ::stdweb::unstable::TryFrom< #name #ty_generics > for ::stdweb::Reference #where_clause {
            type Error = ::stdweb::unstable::Void;

            #[inline]
            fn try_from( value: #name #ty_generics ) -> Result< Self, Self::Error > {
                Ok( value.0 )
            }
        }

        impl #impl_generics ::stdweb::unstable::TryFrom< ::stdweb::Reference > for #name #ty_generics #where_clause {
            type Error = ::stdweb::private::ConversionError;

            #[inline]
            fn try_from( reference: ::stdweb::Reference ) -> Result< Self, Self::Error > {
                reference.downcast()
                    .ok_or_else( || ::stdweb::private::ConversionError::Custom( "reference is of a different type".into() ) )
            }
        }

        impl< '_r, #generics_params > ::stdweb::unstable::TryFrom< &'_r ::stdweb::Reference > for #name #ty_generics #where_clause {
            type Error = ::stdweb::private::ConversionError;

            #[inline]
            fn try_from( reference: &::stdweb::Reference ) -> Result< Self, Self::Error > {
                use ::stdweb::unstable::TryInto;
                reference.clone().try_into()
            }
        }

        impl #impl_generics ::stdweb::unstable::TryFrom< ::stdweb::Value > for #name #ty_generics #where_clause {
            type Error = ::stdweb::private::ConversionError;

            #[inline]
            fn try_from( value: ::stdweb::Value ) -> Result< Self, Self::Error > {
                use ::stdweb::unstable::TryInto;
                let reference: ::stdweb::Reference = value.try_into()?;
                reference.downcast()
                    .ok_or_else( || ::stdweb::private::ConversionError::Custom( "reference is of a different type".into() ) )
            }
        }

        impl< '_r, #generics_params > ::stdweb::unstable::TryFrom< &'_r ::stdweb::Value > for #name #ty_generics #where_clause {
            type Error = ::stdweb::private::ConversionError;

            #[inline]
            fn try_from( value: &::stdweb::Value ) -> Result< Self, Self::Error > {
                use ::stdweb::unstable::TryInto;
                let reference: &::stdweb::Reference =
                    value.as_reference()
                    .ok_or_else( || ::stdweb::private::ConversionError::Custom( "not a reference".into() ) )?;

                reference.try_into()
            }
        }

        impl #impl_generics ::stdweb::private::JsSerializable for #name #ty_generics #where_clause {
            #[inline]
            fn into_js< 'a >( &'a self, arena: &'a ::stdweb::private::PreallocatedArena ) -> ::stdweb::private::SerializedValue< 'a > {
                self.0.into_js( arena )
            }

            #[inline]
            fn memory_required( &self ) -> usize {
                ::stdweb::Reference::memory_required( &self.0 )
            }
        }

        impl #impl_generics ::stdweb::private::JsSerializableOwned for #name #ty_generics #where_clause {
            #[inline]
            fn into_js_owned< '_a >( value: &'_a mut Option< Self >, arena: &'_a ::stdweb::private::PreallocatedArena ) -> ::stdweb::private::SerializedValue< '_a > {
                ::stdweb::private::JsSerializable::into_js( value.as_ref().unwrap(), arena )
            }

            #[inline]
            fn memory_required_owned( &self ) -> usize {
                ::stdweb::private::JsSerializable::memory_required( self )
            }
        }

        impl< '_r, #generics_params > ::stdweb::private::JsSerializableOwned for &'_r #name #ty_generics #where_clause {
            #[inline]
            fn into_js_owned< '_a >( value: &'_a mut Option< Self >, arena: &'_a ::stdweb::private::PreallocatedArena ) -> ::stdweb::private::SerializedValue< '_a > {
                ::stdweb::private::JsSerializable::into_js( value.unwrap(), arena )
            }

            #[inline]
            fn memory_required_owned( &self ) -> usize {
                ::stdweb::private::JsSerializable::memory_required( *self )
            }
        }
    };

    expanded.into()
}
