//! Typed key/value settings stored as JSON.

use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::Store;
use crate::error::Result;

impl Store {
    /// Reads a setting. `Ok(None)` when the key is missing; [`crate::StoreError::Json`] when
    /// the stored value does not decode as `T`.
    pub fn setting<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let raw: Option<String> = self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
            .optional()?;
        match raw {
            Some(raw) => Ok(Some(serde_json::from_str(&raw)?)),
            None => Ok(None),
        }
    }

    /// Reads a setting, giving `default` when the key is missing **or** the stored value no
    /// longer decodes as `T` (for example after the setting's type changed). Only SQL
    /// errors are reported.
    pub fn setting_or<T: DeserializeOwned>(&self, key: &str, default: T) -> Result<T> {
        let raw: Option<String> = self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
            .optional()?;
        Ok(raw
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or(default))
    }

    /// Writes a setting (insert or replace), encoded as JSON.
    pub fn set_setting<T: Serialize + ?Sized>(&mut self, key: &str, value: &T) -> Result<()> {
        let raw = serde_json::to_string(value)?;
        let now = self.now();
        self.conn.execute(
            "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                 updated_at = excluded.updated_at",
            params![key, raw, now],
        )?;
        Ok(())
    }

    /// Deletes a setting; returns whether it existed.
    pub fn remove_setting(&mut self, key: &str) -> Result<bool> {
        Ok(self
            .conn
            .execute("DELETE FROM settings WHERE key = ?1", [key])?
            > 0)
    }

    /// All setting keys, sorted.
    pub fn setting_keys(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached("SELECT key FROM settings ORDER BY key")?;
        let keys = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(keys)
    }
}
