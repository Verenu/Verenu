use anyhow::Result;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, MutexGuard};

mod cleanup_cache;
mod contexts;
mod dictionary;
mod insights;
mod pricing;
mod schema;
mod snippets;
mod sub_apps;
mod transcriptions;
mod validation;

pub use cleanup_cache::*;
pub use contexts::*;
pub use dictionary::*;
pub use insights::*;
pub use pricing::*;
pub use schema::*;
pub use snippets::*;
pub use sub_apps::*;
pub use transcriptions::*;
pub use validation::*;

pub type Db = Arc<Mutex<Connection>>;

fn lock_conn(db: &Db) -> Result<MutexGuard<'_, Connection>> {
    db.lock()
        .map_err(|_| anyhow::anyhow!("Database lock was poisoned"))
}

fn query_all<T>(
    conn: &Connection,
    sql: &str,
    parameters: impl rusqlite::Params,
    map: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> Result<Vec<T>> {
    conn.prepare(sql)?
        .query_map(parameters, map)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CreatedRecordMeta {
    pub id: i64,
    pub created_at: String,
}

#[cfg(test)]
mod autolearn_context_tests;
#[cfg(test)]
mod tests;
