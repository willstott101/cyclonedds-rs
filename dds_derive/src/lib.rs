/*
    Copyright 2021 Sojan James

    Licensed under the Apache License, Version 2.0 (the "License");
    you may not use this file except in compliance with the License.
    You may obtain a copy of the License at

        http://www.apache.org/licenses/LICENSE-2.0

    Unless required by applicable law or agreed to in writing, software
    distributed under the License is distributed on an "AS IS" BASIS,
    WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
    See the License for the specific language governing permissions and
    limitations under the License.
*/

// Rust deserializer for CycloneDDS. (proc macro)
// See discussion at https://github.com/eclipse-cyclonedds/cyclonedds/issues/830

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{parse_macro_input, Field, Ident};

#[proc_macro_derive(TopicFixedSize, attributes(topic_key, topic_key_enum))]
pub fn derive_topic_fixed_size(item: TokenStream) -> TokenStream {
    derive_topic_impl(item, true)
}

#[proc_macro_derive(Topic, attributes(topic_key, topic_key_enum))]
pub fn derive_topic(item: TokenStream) -> TokenStream {
    derive_topic_impl(item, false)
}

fn derive_topic_impl(item: TokenStream, is_fixed_size: bool) -> TokenStream {
    let topic_struct = parse_macro_input!(item as syn::ItemStruct);

    let mut ts = build_key_holder_struct(&topic_struct);
    let ts2 = create_keyhash_functions(&topic_struct, is_fixed_size);
    let ts3 = create_topic_functions(&topic_struct);

    ts.extend(ts2);
    ts.extend(ts3);

    //println!("KEYHOLDER:{:?}",ts.clone().to_string());
    ts
}

///Create a key holder struct from the given struct. The key
///fields will be included in this structure. The structure
///will be empty if there are no key fields.
fn build_key_holder_struct(item: &syn::ItemStruct) -> TokenStream {
    let key_holder_struct = item;

    let mut holder_name = key_holder_struct.ident.to_string();
    let fields = &key_holder_struct.fields;
    holder_name.push_str("KeyHolder_");
    let holder_name = Ident::new(&holder_name, Span::call_site());
    //key_holder_struct.ident = Ident::new(&holder_name,Span::call_site());

    let mut field_idents = Vec::new();
    let mut field_types = Vec::new();
    let mut clone_or_into = Vec::new();
    let mut ref_or_value = Vec::new();
    let mut contained_types = Vec::new();
    let mut variable_length = false;

    for field in fields {
        if is_key(field) {
            field_idents.push(field.ident.as_ref().unwrap().clone());
            if is_primitive(field) || is_key_enum(field) {
                field_types.push(field.ty.clone());
                clone_or_into.push(quote! {clone()});
                ref_or_value.push(quote! {});
                if !variable_length {
                    variable_length = is_variable_length(field);
                }
            } else {
                match field.ty.clone() {
                    syn::Type::Path(mut type_path) => {
                        // if the key is another structure (not a primitive),
                        // there should be a key holder structure for it.
                        // Change the type
                        let last_segment = type_path.path.segments.last_mut().unwrap();
                        let mut ident_string = last_segment.ident.to_string();
                        ident_string.push_str("KeyHolder_");
                        let new_ident = Ident::new(&ident_string, Span::call_site());
                        //replace the ident with the new name
                        last_segment.ident = new_ident;
                        contained_types.push(syn::Type::Path(type_path.clone()));
                        field_types.push(syn::Type::Path(type_path));
                        clone_or_into.push(quote! {into()});
                        ref_or_value.push(quote! { &});
                    }
                    syn::Type::Array(type_arr) => {
                        if let syn::Type::Path(array_type_path) = *type_arr.elem {
                            if is_primitive_type_path(&array_type_path) {
                                field_types.push(field.ty.clone());
                                clone_or_into.push(quote! {clone()});
                                ref_or_value.push(quote! {});
                            } else {
                                panic!("Only primitive arrays are supported as keys");
                            }
                        } else {
                            panic!("Unsupported type for array");
                        }
                    }
                    _ => {
                        panic!("Keys need to be primitives, Path or array of primitives or Path");
                    }
                }
            }
        }
    }

    let item_ident = &item.ident;
    //println!("Filtered fields:{:?}", &filtered_fields);

    let ts = quote! {
        #[derive(Default, Deserialize, Serialize, PartialEq, Clone)]
        struct #holder_name {
            #(#field_idents:#field_types,)*
        }

        impl From<& #item_ident> for #holder_name {
            fn from(source: & #item_ident) -> Self {
                Self {
                    #(#field_idents : (#ref_or_value source.#field_idents). #clone_or_into ,)*
                }
            }
        }

        impl #holder_name {
            const fn is_variable_length() -> bool {
                if !#variable_length {
                    #(#contained_types :: is_variable_length()||)*  false
                } else {
                    true
                }
            }
        }

    };

    ts.into()
}

// create the keyhash methods for this type
fn create_keyhash_functions(item: &syn::ItemStruct, is_fixed_size: bool) -> TokenStream {
    let topic_key_ident = &item.ident;
    let topic_key_holder_ident = quote::format_ident!("{}KeyHolder_", &item.ident);

    let ts = quote! {
        impl TopicType for #topic_key_ident {
            /// return the cdr encoding for the key. The encoded string includes the four byte
            /// encapsulation string.
            fn key_cdr(&self) -> Vec<u8> {
                let holder_struct : #topic_key_holder_ident = self.into();

                let encoded = cdr::serialize::<_, _, cdr::CdrBe>(&holder_struct, cdr::Infinite).expect("Unable to serialize key");
               encoded
            }

            fn is_fixed_size() -> bool {
                #is_fixed_size
            }

            fn has_key() -> bool {
                if std::mem::size_of::<#topic_key_holder_ident>() > 0 {
                    true
                } else {
                    false
                }
            }

            fn force_md5_keyhash() -> bool {
                 #topic_key_holder_ident::is_variable_length()
            }
        }
    };

    ts.into()
}

fn create_topic_functions(item: &syn::ItemStruct) -> TokenStream {
    let topic_key_ident = &item.ident;

    let ts = quote! {
        impl #topic_key_ident {
            /// Create a topic using of this Type specifying the topic name
            ///
            /// # Arguments
            ///
            /// * `participant` - The participant handle onto which this topic should be created
            /// * `name` - The name of the topic
            /// * `maybe_qos` - A QoS structure for this topic.  The Qos is optional
            /// * `maybe_listener` - A listener to use on this topic. The listener is optional
            ///
            pub fn create_topic_with_name(
                participant: &DdsParticipant,
                name: &str,
                maybe_qos: Option<DdsQos>,
                maybe_listener: Option<DdsListener>,
            ) -> Result<DdsTopic::<Self>, DDSError> {
                DdsTopic::<Self>::create(participant,name, maybe_qos,maybe_listener)
            }

            /// Create a topic of this Type using the default topic name. The default topic
            /// name is provided by the Self::topic_name function.
            /// # Arguments
            ///
            /// * `participant` - The participant handle onto which this topic should be created
            /// * `maybe_topic_prefix` - An additional prefix to be added to the topic name. This can be None
            /// * `maybe_qos` - A QoS structure for this topic.  The Qos is optional
            /// * `maybe_listener` - A listener to use on this topic. The listener is optional
            ///
            pub fn create_topic(
                participant: &DdsParticipant,
                maybe_topic_prefix: Option<&str>,
                maybe_qos: Option<DdsQos>,
                maybe_listener: Option<DdsListener>,
            ) -> Result<DdsTopic::<Self>, DDSError> {
                let name = #topic_key_ident::topic_name(maybe_topic_prefix);
                DdsTopic::<Self>::create(participant,&name, maybe_qos,maybe_listener)
            }

            /// Create a sample buffer for storing an array of samples
            /// You can pass the sample buffer into a read to read multiple
            /// samples. Multiple samples are useful when you have one or more
            /// keys in your topic structure. Each value of the key will result in
            /// the storage of another sample.
            pub fn create_sample_buffer(len: usize) -> SampleBuffer<#topic_key_ident> {
                SampleBuffer::new(len)
            }
        }
    };

    ts.into()
}

/*
fn struct_has_key(it: &ItemStruct) -> bool {
    for field in &it.fields {
        if is_key(field) {
            return true
        }
    }
    false
}
*/

fn is_key(field: &Field) -> bool {
    for attr in &field.attrs {
        if let Some(ident) = attr.path.get_ident() {
            if ident == "topic_key" || ident == "topic_key_enum" {
                return true;
            }
        }
    }
    false
}

// There is no way to find out if the field is an enum or a struct,
// so we need a special marker to indicate key enums
// which we will treat like primitives.
fn is_key_enum(field: &Field) -> bool {
    for attr in &field.attrs {
        if let Some(ident) = attr.path.get_ident() {
            if ident == "topic_key_enum" {
                return true;
            }
        }
    }
    false
}

fn is_primitive_type_path(type_path: &syn::TypePath) -> bool {
    if type_path.path.is_ident("bool")
        || type_path.path.is_ident("i8")
        || type_path.path.is_ident("i16")
        || type_path.path.is_ident("i32")
        || type_path.path.is_ident("i64")
        || type_path.path.is_ident("i128")
        || type_path.path.is_ident("isize")
        || type_path.path.is_ident("u8")
        || type_path.path.is_ident("u16")
        || type_path.path.is_ident("u32")
        || type_path.path.is_ident("u64")
        || type_path.path.is_ident("u128")
        || type_path.path.is_ident("usize")
        || type_path.path.is_ident("f32")
        || type_path.path.is_ident("f64")
        || type_path.path.is_ident("String")
    {
        true
    } else {
        false
    }
}

// check if a field is of a primitive type. We assume anything not primitive
// is a struct
fn is_primitive(field: &Field) -> bool {
    if let syn::Type::Path(type_path) = &field.ty {
        is_primitive_type_path(type_path)
    } else {
        false
    }
}

// Is the length of the underlying type variable. This is needed
// According to the DDSI RTPS spec, the potential length of a field
// must be checked to decide whether to use md5 checksum for the key
// hash.  If a String (or Vec) is used as a key_field, then the
// length is variable.
fn is_variable_length(field: &Field) -> bool {
    if let syn::Type::Path(type_path) = &field.ty {
        if type_path.path.is_ident("Vec") || type_path.path.is_ident("String") {
            true
        } else {
            false
        }
    } else {
        false
    }
}
