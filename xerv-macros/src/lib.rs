//! Procedural macros for XERV.
//!
//! This crate provides the following macros:
//! - `#[xerv::node]` - Define a node with namespace and name
//! - `#[xerv::schema]` - Derive schema metadata with stable layout
//! - `#[xerv::pipeline]` - Define a pipeline controller

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, Ident, ItemFn, LitStr, parse_macro_input};

/// Marks a function as a XERV node.
///
/// # Attributes
/// - `namespace`: The node namespace (e.g., "std", "plugins")
/// - `name`: The node name (e.g., "switch", "fraud_model")
///
/// # Example
///
/// ```ignore
/// #[xerv::node(namespace = "std", name = "slack_post")]
/// pub async fn run(
///     ctx: Context,
///     cfg: SlackConfig,
///     input: AlertInput
/// ) -> Result<NodeOutput> {
///     // Node logic here
///     Ok(NodeOutput::out(ptr))
/// }
/// ```
#[proc_macro_attribute]
pub fn node(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as NodeArgs);
    let input_fn = parse_macro_input!(item as ItemFn);

    let expanded = expand_node(args, input_fn);
    TokenStream::from(expanded)
}

/// Derives schema metadata for a struct.
///
/// This macro:
/// - Adds `#[repr(C)]` for stable memory layout
/// - Generates compile-time offset constants
/// - Implements the `Schema` trait
/// - Registers the schema in the global registry
///
/// # Attributes
/// - `version`: Schema version string (default: "v1")
///
/// # Example
///
/// ```ignore
/// #[xerv::schema(version = "v1")]
/// #[derive(Archive, Serialize, Deserialize)]
/// pub struct FraudResult {
///     pub score: f32,
///     pub risk_level: u8,
///     pub flags: u32,
/// }
/// ```
#[proc_macro_attribute]
pub fn schema(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as SchemaArgs);
    let input = parse_macro_input!(item as DeriveInput);

    let expanded = expand_schema(args, input);
    TokenStream::from(expanded)
}

/// Marks a struct as a XERV pipeline controller.
///
/// # Attributes
/// - `name`: The pipeline name (e.g., "order_processor_v1")
///
/// # Example
///
/// ```ignore
/// #[xerv::pipeline(name = "order_processor_v1")]
/// pub struct OrderPipeline {
///     config: ECommerceConfig,
///     state: PipelineState,
/// }
/// ```
#[proc_macro_attribute]
pub fn pipeline(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as PipelineArgs);
    let input = parse_macro_input!(item as DeriveInput);

    let expanded = expand_pipeline(args, input);
    TokenStream::from(expanded)
}

// =============================================================================
// Node Macro Implementation
// =============================================================================

struct NodeArgs {
    namespace: String,
    name: String,
}

impl syn::parse::Parse for NodeArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut namespace = String::from("custom");
        let mut name = String::new();

        while !input.is_empty() {
            let ident: Ident = input.parse()?;
            input.parse::<syn::Token![=]>()?;

            match ident.to_string().as_str() {
                "namespace" => {
                    let lit: LitStr = input.parse()?;
                    namespace = lit.value();
                }
                "name" => {
                    let lit: LitStr = input.parse()?;
                    name = lit.value();
                }
                other => {
                    return Err(syn::Error::new(
                        ident.span(),
                        format!("unknown attribute: {}", other),
                    ));
                }
            }

            if !input.is_empty() {
                input.parse::<syn::Token![,]>()?;
            }
        }

        if name.is_empty() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "missing required attribute: name",
            ));
        }

        Ok(NodeArgs { namespace, name })
    }
}

fn expand_node(args: NodeArgs, input_fn: ItemFn) -> TokenStream2 {
    let _fn_name = &input_fn.sig.ident;
    let fn_vis = &input_fn.vis;
    let fn_block = &input_fn.block;
    let fn_attrs = &input_fn.attrs;

    let struct_name = format_ident!("{}Node", to_pascal_case(&args.name));
    let factory_name = format_ident!("{}Factory", to_pascal_case(&args.name));
    let namespace = &args.namespace;
    let name = &args.name;
    let full_name = format!("{}::{}", namespace, name);

    // Extract parameter types from the function signature
    let params: Vec<_> = input_fn.sig.inputs.iter().collect();

    // We expect: (ctx: Context, cfg: ConfigType, input: InputType)
    let (config_type, input_type) = if params.len() >= 3 {
        let cfg_param = &params[1];
        let input_param = &params[2];

        let cfg_ty = extract_type(cfg_param);
        let input_ty = extract_type(input_param);

        (cfg_ty, input_ty)
    } else {
        (quote!(serde_yaml::Value), quote!(()))
    };

    quote! {
        #(#fn_attrs)*
        #fn_vis struct #struct_name {
            config: #config_type,
        }

        impl #struct_name {
            /// Create a new node instance with the given configuration.
            pub fn new(config: #config_type) -> Self {
                Self { config }
            }
        }

        impl xerv_core::traits::Node for #struct_name {
            fn info(&self) -> xerv_core::traits::NodeInfo {
                xerv_core::traits::NodeInfo::new(#namespace, #name)
                    .with_description(concat!("Node: ", #full_name))
            }

            fn execute<'a>(
                &'a self,
                ctx: xerv_core::traits::Context,
                inputs: std::collections::HashMap<String, xerv_core::types::RelPtr<()>>,
            ) -> xerv_core::traits::NodeFuture<'a> {
                Box::pin(async move {
                    let cfg = &self.config;
                    let input = inputs.get("in").cloned().unwrap_or_else(|| {
                        xerv_core::types::RelPtr::null()
                    });

                    // User's node logic
                    let inner_fn = |ctx: xerv_core::traits::Context,
                                    cfg: &#config_type,
                                    input: xerv_core::types::RelPtr<#input_type>|
                                    #fn_block;

                    inner_fn(ctx, cfg, input.cast())
                })
            }
        }

        /// Factory for creating #struct_name instances.
        #fn_vis struct #factory_name;

        impl xerv_core::traits::NodeFactory for #factory_name {
            fn node_type(&self) -> &str {
                #full_name
            }

            fn create(&self, config: &serde_yaml::Value) -> xerv_core::error::Result<Box<dyn xerv_core::traits::Node>> {
                let cfg: #config_type = serde_yaml::from_value(config.clone())
                    .map_err(|e| xerv_core::error::XervError::ConfigValue {
                        field: "config".to_string(),
                        cause: e.to_string(),
                    })?;
                Ok(Box::new(#struct_name::new(cfg)))
            }
        }
    }
}

fn extract_type(param: &syn::FnArg) -> TokenStream2 {
    match param {
        syn::FnArg::Typed(pat_type) => {
            let ty = &pat_type.ty;
            quote!(#ty)
        }
        _ => quote!(()),
    }
}

fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        })
        .collect()
}

// =============================================================================
// Schema Macro Implementation
// =============================================================================

struct SchemaArgs {
    version: String,
}

impl syn::parse::Parse for SchemaArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut version = String::from("v1");

        while !input.is_empty() {
            let ident: Ident = input.parse()?;
            input.parse::<syn::Token![=]>()?;

            match ident.to_string().as_str() {
                "version" => {
                    let lit: LitStr = input.parse()?;
                    version = lit.value();
                }
                other => {
                    return Err(syn::Error::new(
                        ident.span(),
                        format!("unknown attribute: {}", other),
                    ));
                }
            }

            if !input.is_empty() {
                input.parse::<syn::Token![,]>()?;
            }
        }

        Ok(SchemaArgs { version })
    }
}

fn expand_schema(args: SchemaArgs, mut input: DeriveInput) -> TokenStream2 {
    let name = &input.ident;
    let version = &args.version;
    let version_num: u32 = version.trim_start_matches('v').parse().unwrap_or(1);
    let hash_input = format!("{}@{}", name, version);

    // Add #[repr(C)] attribute
    let repr_c = syn::parse_quote!(#[repr(C)]);
    input.attrs.push(repr_c);

    // Extract field information
    let fields = match &input.data {
        Data::Struct(data_struct) => match &data_struct.fields {
            Fields::Named(fields) => &fields.named,
            _ => {
                return syn::Error::new_spanned(
                    &input,
                    "xerv::schema only supports structs with named fields",
                )
                .to_compile_error();
            }
        },
        _ => {
            return syn::Error::new_spanned(&input, "xerv::schema only supports structs")
                .to_compile_error();
        }
    };

    // Generate offset constants
    let offset_consts: Vec<_> = fields
        .iter()
        .map(|field| {
            let field_name = field.ident.as_ref().unwrap();
            let const_name = format_ident!("OFFSET_{}", field_name.to_string().to_uppercase());

            quote! {
                /// Byte offset of the `#field_name` field.
                pub const #const_name: usize = std::mem::offset_of!(#name, #field_name);
            }
        })
        .collect();

    // Generate field info for Schema trait
    let field_infos: Vec<_> = fields
        .iter()
        .map(|field| {
            let field_name = field.ident.as_ref().unwrap().to_string();
            let field_type = &field.ty;
            let type_name = quote!(#field_type).to_string();

            quote! {
                xerv_core::traits::FieldInfo::new(#field_name, #type_name)
                    .with_offset(std::mem::offset_of!(#name, #field_name))
                    .with_size(std::mem::size_of::<#field_type>())
            }
        })
        .collect();

    let vis = &input.vis;
    let attrs = &input.attrs;
    let generics = &input.generics;
    let data = &input.data;

    // Reconstruct the struct with repr(C)
    let struct_def = match data {
        Data::Struct(data_struct) => {
            let fields = &data_struct.fields;
            quote! {
                #(#attrs)*
                #vis struct #name #generics #fields
            }
        }
        _ => unreachable!(),
    };

    // Generate a hash from the struct name and version
    let hash_value = simple_hash(&hash_input);

    quote! {
        #struct_def

        impl #name {
            #(#offset_consts)*
        }

        impl xerv_core::traits::Schema for #name {
            fn type_info() -> xerv_core::traits::TypeInfo {
                xerv_core::traits::TypeInfo::new(stringify!(#name), #version_num)
                    .with_hash(#hash_value)
                    .with_size(std::mem::size_of::<#name>())
                    .with_alignment(std::mem::align_of::<#name>())
                    .with_fields(vec![#(#field_infos),*])
                    .stable()
            }
        }
    }
}

/// Simple FNV-1a hash for generating schema hashes at compile time.
fn simple_hash(s: &str) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET;
    for byte in s.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

// =============================================================================
// Pipeline Macro Implementation
// =============================================================================

struct PipelineArgs {
    name: String,
}

impl syn::parse::Parse for PipelineArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut name = String::new();

        while !input.is_empty() {
            let ident: Ident = input.parse()?;
            input.parse::<syn::Token![=]>()?;

            match ident.to_string().as_str() {
                "name" => {
                    let lit: LitStr = input.parse()?;
                    name = lit.value();
                }
                other => {
                    return Err(syn::Error::new(
                        ident.span(),
                        format!("unknown attribute: {}", other),
                    ));
                }
            }

            if !input.is_empty() {
                input.parse::<syn::Token![,]>()?;
            }
        }

        if name.is_empty() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "missing required attribute: name",
            ));
        }

        Ok(PipelineArgs { name })
    }
}

fn expand_pipeline(args: PipelineArgs, input: DeriveInput) -> TokenStream2 {
    let struct_name = &input.ident;
    let pipeline_name = &args.name;
    let vis = &input.vis;
    let attrs = &input.attrs;
    let generics = &input.generics;

    let fields = match &input.data {
        Data::Struct(data_struct) => match &data_struct.fields {
            Fields::Named(fields) => &fields.named,
            _ => {
                return syn::Error::new_spanned(
                    &input,
                    "xerv::pipeline only supports structs with named fields",
                )
                .to_compile_error();
            }
        },
        _ => {
            return syn::Error::new_spanned(&input, "xerv::pipeline only supports structs")
                .to_compile_error();
        }
    };

    // Look for config and state fields
    let has_config = fields
        .iter()
        .any(|f| f.ident.as_ref().is_some_and(|i| i == "config"));
    let has_state = fields
        .iter()
        .any(|f| f.ident.as_ref().is_some_and(|i| i == "state"));

    let config_accessor = if has_config {
        quote! {
            /// Get a reference to the pipeline configuration.
            pub fn config(&self) -> &impl std::any::Any {
                &self.config
            }
        }
    } else {
        quote!()
    };

    let state_accessor = if has_state {
        quote! {
            /// Get the current pipeline state.
            pub fn state(&self) -> &xerv_core::traits::PipelineState {
                &self.state
            }

            /// Set the pipeline state.
            pub fn set_state(&mut self, state: xerv_core::traits::PipelineState) {
                self.state = state;
            }
        }
    } else {
        quote!()
    };

    quote! {
        #(#attrs)*
        #vis struct #struct_name #generics {
            #fields
        }

        impl #struct_name {
            /// Get the pipeline name.
            pub const fn pipeline_name() -> &'static str {
                #pipeline_name
            }

            #config_accessor
            #state_accessor
        }
    }
}
