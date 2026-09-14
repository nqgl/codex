use super::*;
use crate::ThreadMonitor;
use codex_protocol::ThreadId;

impl StateRuntime {
    pub async fn upsert_thread_monitor(
        &self,
        thread_id: ThreadId,
        monitor: &ThreadMonitor,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO thread_monitors (thread_id, name, command, cwd, trusted, running)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(thread_id, name) DO UPDATE SET
                 command = excluded.command,
                 cwd = excluded.cwd,
                 trusted = excluded.trusted,
                 running = excluded.running",
        )
        .bind(thread_id.to_string())
        .bind(&monitor.name)
        .bind(&monitor.command)
        .bind(monitor.cwd.display().to_string())
        .bind(monitor.trusted)
        .bind(monitor.running)
        .execute(self.pool.as_ref())
        .await?;
        Ok(())
    }

    pub async fn list_thread_monitors(
        &self,
        thread_id: ThreadId,
    ) -> anyhow::Result<Vec<ThreadMonitor>> {
        let rows = sqlx::query(
            "SELECT name, command, cwd, trusted, running
             FROM thread_monitors
             WHERE thread_id = ?
             ORDER BY name",
        )
        .bind(thread_id.to_string())
        .fetch_all(self.pool.as_ref())
        .await?;
        rows.iter().map(ThreadMonitor::try_from_row).collect()
    }

    pub async fn delete_thread_monitor(
        &self,
        thread_id: ThreadId,
        name: &str,
    ) -> anyhow::Result<bool> {
        Ok(
            sqlx::query("DELETE FROM thread_monitors WHERE thread_id = ? AND name = ?")
                .bind(thread_id.to_string())
                .bind(name)
                .execute(self.pool.as_ref())
                .await?
                .rows_affected()
                > 0,
        )
    }
}
