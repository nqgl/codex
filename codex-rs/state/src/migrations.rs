use std::borrow::Cow;

use sqlx::migrate::Migrate;
use sqlx::migrate::Migrator;
use sqlx_sqlite::SqlitePool;

pub(crate) static STATE_MIGRATOR: Migrator = sqlx_macros::migrate!("./migrations");
pub(crate) static MONITOR_MIGRATOR: Migrator = sqlx_macros::migrate!("./monitor_migrations");
const MONITOR_MIGRATION_TABLE: &str = "_codex_monitor_migrations";
pub(crate) static LOGS_MIGRATOR: Migrator = sqlx_macros::migrate!("./logs_migrations");
pub(crate) static GOALS_MIGRATOR: Migrator = sqlx_macros::migrate!("./goals_migrations");
pub(crate) static MEMORIES_MIGRATOR: Migrator = sqlx_macros::migrate!("./memory_migrations");
pub(crate) static QUEUE_MIGRATOR: Migrator = sqlx_macros::migrate!("./queue_migrations");
pub(crate) static THREAD_HISTORY_MIGRATOR: Migrator =
    sqlx_macros::migrate!("./thread_history_migrations");

/// Allow an older Codex binary to open a database that has already been
/// migrated by a newer binary running in parallel.
///
/// We intentionally ignore applied migration versions that are newer than the
/// embedded migration set. Known migration versions are still validated by
/// checksum, so this only relaxes the "database is ahead of me" case.
fn runtime_migrator(base: &'static Migrator) -> Migrator {
    Migrator {
        migrations: Cow::Borrowed(base.migrations.as_ref()),
        ignore_missing: true,
        locking: base.locking,
        no_tx: base.no_tx,
        table_name: base.table_name.clone(),
        create_schemas: base.create_schemas.clone(),
    }
}

pub(crate) fn runtime_state_migrator() -> Migrator {
    runtime_migrator(&STATE_MIGRATOR)
}

pub(crate) fn runtime_monitor_migrator() -> Migrator {
    let mut migrator = runtime_migrator(&MONITOR_MIGRATOR);
    migrator.table_name = Cow::Borrowed(MONITOR_MIGRATION_TABLE);
    migrator
}

pub(crate) fn runtime_logs_migrator() -> Migrator {
    runtime_migrator(&LOGS_MIGRATOR)
}

pub(crate) fn runtime_goals_migrator() -> Migrator {
    runtime_migrator(&GOALS_MIGRATOR)
}

pub(crate) fn runtime_memories_migrator() -> Migrator {
    runtime_migrator(&MEMORIES_MIGRATOR)
}

pub(crate) fn runtime_queue_migrator() -> Migrator {
    runtime_migrator(&QUEUE_MIGRATOR)
}

// The paginated history projector will call this when it takes ownership of opening the database.
#[allow(dead_code)]
pub(crate) fn runtime_thread_history_migrator() -> Migrator {
    runtime_migrator(&THREAD_HISTORY_MIGRATOR)
}

pub(crate) async fn repair_legacy_recency_migration_version(
    pool: &SqlitePool,
    migrator: &Migrator,
) -> anyhow::Result<()> {
    let Some(recency_migration) = migrator
        .migrations
        .iter()
        .find(|migration| migration.version == 39)
    else {
        return Ok(());
    };
    let migrations_table_exists = sqlx::query_scalar::<_, i64>(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_optional(pool)
    .await?
    .is_some();
    if !migrations_table_exists {
        return Ok(());
    }

    let legacy_recency_needs_repair = sqlx::query_scalar::<_, i64>(
        r#"
SELECT 1
FROM _sqlx_migrations
WHERE version = ?
  AND checksum = ?
  AND NOT EXISTS (
      SELECT 1 FROM _sqlx_migrations WHERE version = ?
  )
        "#,
    )
    .bind(38_i64)
    .bind(recency_migration.checksum.as_ref())
    .bind(recency_migration.version)
    .fetch_optional(pool)
    .await?
    .is_some();
    if !legacy_recency_needs_repair {
        return Ok(());
    }

    sqlx::query(
        r#"
UPDATE _sqlx_migrations
SET version = ?, description = ?
WHERE version = ?
  AND checksum = ?
  AND NOT EXISTS (
      SELECT 1 FROM _sqlx_migrations WHERE version = ?
  )
        "#,
    )
    .bind(recency_migration.version)
    .bind(recency_migration.description.as_ref())
    .bind(38_i64)
    .bind(recency_migration.checksum.as_ref())
    .bind(recency_migration.version)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn repair_divergent_migration_versions(
    pool: &SqlitePool,
    migrator: &Migrator,
) -> anyhow::Result<()> {
    let migrations_table_exists = sqlx::query_scalar::<_, i64>(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_optional(pool)
    .await?
    .is_some();
    if !migrations_table_exists {
        return Ok(());
    }
    // Acquire the writer slot before inspecting/creating the custom ledger. A deferred
    // transaction can fail its read-to-write upgrade immediately despite busy_timeout.
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    transaction
        .ensure_migrations_table(MONITOR_MIGRATION_TABLE)
        .await?;

    // These are the three released custom monitor layouts. Transfer only exact known checksums;
    // unrelated or failed migration records must still fail normal SQLx validation.
    // A separate ledger leaves all future upstream version numbers available.
    for (legacy_versions, version) in [
        ([48_i64, 51_i64, 54_i64], 54_i64),
        ([49_i64, 52_i64, 55_i64], 55_i64),
    ] {
        let Some(migration) = MONITOR_MIGRATOR
            .migrations
            .iter()
            .find(|m| m.version == version)
        else {
            continue;
        };
        for legacy in legacy_versions {
            sqlx::query(
                "INSERT INTO _codex_monitor_migrations
                 (version, description, installed_on, success, checksum, execution_time)
                 SELECT ?, ?, installed_on, success, checksum, execution_time FROM _sqlx_migrations
                 WHERE version = ? AND checksum = ?
                 ON CONFLICT(version) DO NOTHING",
            )
            .bind(version)
            .bind(migration.description.as_ref())
            .bind(legacy)
            .bind(migration.checksum.as_ref())
            .execute(&mut *transaction)
            .await?;
            sqlx::query(
                "DELETE FROM _sqlx_migrations
                 WHERE version = ? AND checksum = ?
                   AND EXISTS (
                     SELECT 1 FROM _codex_monitor_migrations AS monitor
                     WHERE monitor.version = ? AND monitor.checksum = _sqlx_migrations.checksum
                       AND monitor.success = _sqlx_migrations.success
                   )",
            )
            .bind(legacy)
            .bind(migration.checksum.as_ref())
            .bind(version)
            .execute(&mut *transaction)
            .await?;
        }
    }

    // Restore shifted upstream migrations to the versions used by standard Codex builds.
    for (legacy_version, current_version) in [(53_i64, 51_i64), (54_i64, 52_i64), (55_i64, 53_i64)]
    {
        let Some(current_migration) = migrator
            .migrations
            .iter()
            .find(|migration| migration.version == current_version)
        else {
            continue;
        };
        sqlx::query(
            "UPDATE _sqlx_migrations SET version = ?, description = ?
             WHERE version = ? AND checksum = ?
               AND NOT EXISTS (SELECT 1 FROM _sqlx_migrations WHERE version = ?)",
        )
        .bind(current_version)
        .bind(current_migration.description.as_ref())
        .bind(legacy_version)
        .bind(current_migration.checksum.as_ref())
        .bind(current_version)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(())
}

#[cfg(test)]
#[path = "migrations_tests.rs"]
mod tests;
