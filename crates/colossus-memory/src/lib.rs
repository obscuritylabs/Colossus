//! Canonical event-sourced memories and the disposable Tantivy lexical index.

use async_trait::async_trait;
use colossus_contracts::{
    Actor, ActorType, EventClassification, ExecutionContext, MemoryRecord, MemoryScope,
    MemoryStatus, NewEvent,
};
use colossus_ports::{
    EventJournal, ExternalWorkQueue, MemoryIndex, MemoryRepository, SessionRepository, StoreError,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{Arc, Mutex},
};
use tantivy::{
    Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term,
    collector::{Count, TopDocs},
    doc,
    query::{QueryParser, TermQuery},
    schema::{Field, IndexRecordOption, STORED, STRING, Schema, TEXT, Value as _},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

use repository::{MAX_LIST, POSITION_DOCUMENT_ID, adapter, expired, now};

mod repository;
pub use repository::*;

mod index;
pub use index::*;

mod service;
pub use service::*;

#[cfg(test)]
mod tests;
