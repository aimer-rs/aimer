//! A pure byte-migration contract, independent of any adapter.

use crate::error::{StorageError, StorageResult};

/// A versioned transformation for one stored byte value.
pub trait StorageMigration {
    /// Returns the schema version accepted by this migration.
    fn from_version(&self) -> u32;

    /// Returns the schema version produced by this migration.
    fn to_version(&self) -> u32;

    /// Transforms one value without changing its storage key or namespace.
    fn migrate(&self, value: &[u8]) -> StorageResult<Vec<u8>>;
}

/// Applies exact-version migrations in order until `to_version` is reached.
///
/// # Errors
///
/// * [`StorageError::MigrationUnavailable`] when `from_version` is newer than
///   `to_version`, or no migration accepts the version reached so far.
/// * [`StorageError::InvalidMigration`] when a migration does not move strictly
///   forward or overshoots `to_version`.
/// * Whatever a migration itself returns.
pub fn migrate_bytes(
    value: &[u8],
    from_version: u32,
    to_version: u32,
    migrations: &[&dyn StorageMigration],
) -> StorageResult<Vec<u8>> {
    if from_version == to_version {
        return Ok(value.to_vec());
    }
    if from_version > to_version {
        return Err(StorageError::MigrationUnavailable {
            from_version,
            to_version,
        });
    }

    let mut version = from_version;
    let mut migrated = value.to_vec();
    // Every accepted step strictly increases `version`, so this bound can only
    // be reached by a chain longer than the migrations provided.
    for _ in 0..=migrations.len() {
        if version == to_version {
            return Ok(migrated);
        }
        let Some(migration) = migrations
            .iter()
            .find(|migration| migration.from_version() == version)
        else {
            return Err(StorageError::MigrationUnavailable {
                from_version: version,
                to_version,
            });
        };
        if migration.to_version() <= version || migration.to_version() > to_version {
            return Err(StorageError::InvalidMigration {
                from_version: version,
                to_version: migration.to_version(),
            });
        }
        migrated = migration.migrate(&migrated)?;
        version = migration.to_version();
    }

    Err(StorageError::MigrationUnavailable {
        from_version: version,
        to_version,
    })
}
