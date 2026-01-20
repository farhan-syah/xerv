//! Serializable data types for schema registry persistence.

use crate::traits::{FieldInfo, TypeInfo};

/// Serializable schema data for persistence.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct SchemaData {
    pub name: String,
    pub short_name: String,
    pub version: u32,
    pub hash: u64,
    pub size: usize,
    pub alignment: usize,
    pub fields: Vec<FieldData>,
    pub stable_layout: bool,
}

impl SchemaData {
    pub fn from(info: &TypeInfo) -> Self {
        Self {
            name: info.name.clone(),
            short_name: info.short_name.clone(),
            version: info.version,
            hash: info.hash,
            size: info.size,
            alignment: info.alignment,
            fields: info.fields.iter().map(FieldData::from).collect(),
            stable_layout: info.stable_layout,
        }
    }

    pub fn into_type_info(self) -> TypeInfo {
        TypeInfo {
            name: self.name,
            short_name: self.short_name,
            version: self.version,
            hash: self.hash,
            size: self.size,
            alignment: self.alignment,
            fields: self
                .fields
                .into_iter()
                .map(|f| f.into_field_info())
                .collect(),
            stable_layout: self.stable_layout,
        }
    }
}

/// Serializable field data for persistence.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct FieldData {
    name: String,
    type_name: String,
    offset: usize,
    size: usize,
    optional: bool,
}

impl FieldData {
    pub fn from(info: &FieldInfo) -> Self {
        Self {
            name: info.name.clone(),
            type_name: info.type_name.clone(),
            offset: info.offset,
            size: info.size,
            optional: info.optional,
        }
    }

    pub fn into_field_info(self) -> FieldInfo {
        let mut info = FieldInfo::new(self.name, self.type_name)
            .with_offset(self.offset)
            .with_size(self.size);
        if self.optional {
            info = info.optional();
        }
        info
    }
}

/// Serializable family data for persistence.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct FamilyData {
    pub name: String,
    pub versions: Vec<(String, u64)>,
}

/// Top-level registry persistence format.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct RegistryData {
    pub schemas: Vec<SchemaData>,
    pub families: Vec<FamilyData>,
}
