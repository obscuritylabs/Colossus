//! PostgreSQL row models. Domain objects never depend on these database types.
use colossus_cloud::{
    CloudError, CloudResult,
    storage::{EntityKey, EntityKind, EntityValue},
};
use colossus_ports::StoreError;
use diesel::{
    pg::Pg,
    query_builder::{BoxedSqlQuery, SqlQuery},
    sql_types::{BigInt, Text},
};

pub(crate) type Query = BoxedSqlQuery<'static, Pg, SqlQuery>;

#[derive(Clone, diesel::QueryableByName)]
pub(crate) struct Metadata {
    #[diesel(sql_type = Text)]
    pub project_id: String,
    #[diesel(sql_type = Text)]
    pub parent_id: String,
    #[diesel(sql_type = Text)]
    pub id: String,
    #[diesel(sql_type = BigInt)]
    pub revision: i64,
    #[diesel(sql_type = Text)]
    pub audit_hash: String,
}

impl Metadata {
    pub fn key(&self, kind: EntityKind) -> EntityKey {
        EntityKey {
            kind,
            project_id: self.project_id.clone(),
            parent_id: (!self.parent_id.is_empty()).then(|| self.parent_id.clone()),
            id: self.id.clone(),
        }
    }
    pub fn revision(&self) -> CloudResult<u64> {
        let revision = unsigned(self.revision)?;
        if revision == 0 {
            return Err(CloudError::Storage);
        }
        Ok(revision)
    }
}

/// Native columns and bindings shared by the transactional write machinery.
pub(crate) trait TypedRow: diesel::QueryableByName<Pg> + Send + 'static {
    const COLUMNS: &'static [&'static str];
    fn metadata(&self) -> &Metadata;
    fn bind(self, query: Query) -> Query;
}

/// Explicit, compiler-checked field conversion owned by the PostgreSQL adapter.
pub(crate) trait DomainRow: TypedRow {
    fn from_value(metadata: Metadata, value: &EntityValue) -> Result<Self, StoreError>
    where
        Self: Sized;
    fn into_value(self) -> CloudResult<EntityValue>;
}

// Declare native row fields once: Diesel decoding and bound write types share them.
// Additional embedded read fields are computed from normalized child rows.
macro_rules! row {
    ($name:ident { $($field:ident: $rust:ty => $sql:ty),* $(,)? } $(read { $($extra:ident: $extra_type:ty),* $(,)? })?) => {
        #[derive(diesel::QueryableByName)]
        pub(super) struct $name {
            #[diesel(embed)]
            pub metadata: $crate::rows::Metadata,
            $(#[diesel(sql_type = $sql)] pub $field: $rust,)*
            $($(#[diesel(embed)] pub $extra: $extra_type,)*)?
        }
        impl $crate::rows::TypedRow for $name {
            const COLUMNS: &'static [&'static str] = &[$(stringify!($field)),*];
            fn metadata(&self) -> &$crate::rows::Metadata { &self.metadata }
            fn bind(self, query: $crate::rows::Query) -> $crate::rows::Query {
                query$(.bind::<$sql, _>(self.$field))*
            }
        }
    };
}

mod accounts;
mod conversations;
mod inventory;
mod placements;
mod read;
mod settings;
mod write;
pub(crate) use read::{joins, load, selection};
pub(crate) use write::write;

pub(super) fn mismatch() -> StoreError {
    StoreError::Adapter("cloud entity kind mismatch".into())
}
pub(super) fn unsigned(value: i64) -> CloudResult<u64> {
    u64::try_from(value).map_err(|_| CloudError::Storage)
}
pub(super) fn signed(value: u64) -> Result<i64, StoreError> {
    crate::entities::integer(value)
}

pub(super) fn set<T: Ord>(values: Vec<T>) -> CloudResult<std::collections::BTreeSet<T>> {
    // Reject noncanonical storage rather than concealing reordered/duplicate values
    // during domain reconstruction and audit verification.
    if !values.windows(2).all(|pair| pair[0] < pair[1]) {
        return Err(CloudError::Storage);
    }
    Ok(values.into_iter().collect())
}

pub(super) fn payload<T: serde::Serialize>(value: &T) -> Result<serde_json::Value, StoreError> {
    serde_json::to_value(value)
        .map_err(|_| StoreError::Adapter("cloud payload encoding failed".into()))
}
pub(super) fn decode_payload<T: serde::de::DeserializeOwned + serde::Serialize>(
    value: serde_json::Value,
) -> CloudResult<T> {
    let decoded = serde_json::from_value::<T>(value.clone()).map_err(|_| CloudError::Storage)?;
    // Unknown/defaulted fields must not disappear before checking the audit digest.
    if payload(&decoded).map_err(|_| CloudError::Storage)? != value {
        return Err(CloudError::Storage);
    }
    Ok(decoded)
}
