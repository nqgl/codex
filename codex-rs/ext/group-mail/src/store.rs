use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use codex_protocol::ThreadId;
use codex_state::SqliteConfig;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde::Deserialize;
use sqlx::Row;
use sqlx::SqlitePool;
use std::collections::HashSet;
use std::path::Path;

const MAX_MEMBERS: i64 = 32;
const MAX_MESSAGE_BYTES: usize = 4_000;
const MAX_PENDING_BYTES: i64 = 8_000;
const LOW_PENDING_BYTES: i64 = 4_000;
const MAX_PENDING_MESSAGES: i64 = 16;
const ONLINE_LEASE_SECONDS: i64 = 8;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Membership {
    pub group: String,
    pub name: String,
    pub thread_id: ThreadId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemberStatus {
    pub name: String,
    pub online: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SendReceipt {
    pub recipients: Vec<MemberStatus>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingMessage {
    pub id: i64,
    pub sender: String,
    pub body: String,
    pub high: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    High,
    Low,
}

#[derive(Clone)]
pub struct GroupMailStore {
    pool: SqlitePool,
}

impl GroupMailStore {
    pub async fn open(sqlite_home: &Path) -> Result<Self> {
        tokio::fs::create_dir_all(sqlite_home).await?;
        let absolute_home = AbsolutePathBuf::try_from(sqlite_home.to_path_buf())?;
        let sqlite = SqliteConfig::from_sqlite_home(absolute_home);
        let pool = sqlite
            .open_read_write_pool(&sqlite_home.join("group_mail_1.sqlite"))
            .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS members (
                thread_id TEXT PRIMARY KEY,
                group_name TEXT NOT NULL,
                name TEXT NOT NULL,
                owner TEXT,
                seen_at INTEGER,
                UNIQUE(group_name, name)
            )",
        )
        .execute(&pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                recipient TEXT NOT NULL,
                sender TEXT NOT NULL,
                body TEXT NOT NULL,
                high INTEGER NOT NULL CHECK(high IN (0, 1)),
                FOREIGN KEY(recipient) REFERENCES members(thread_id) ON DELETE CASCADE
            )",
        )
        .execute(&pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS messages_recipient_id ON messages(recipient, id)")
            .execute(&pool)
            .await?;
        Ok(Self { pool })
    }

    pub async fn join(&self, thread_id: ThreadId, group: &str, name: &str) -> Result<()> {
        validate_label(group)?;
        validate_label(name)?;
        let mut transaction = self.pool.begin().await?;
        if let Some(existing) =
            sqlx::query("SELECT group_name, name FROM members WHERE thread_id = ?")
                .bind(thread_id.to_string())
                .fetch_optional(&mut *transaction)
                .await?
        {
            let old_group: String = existing.try_get("group_name")?;
            let old_name: String = existing.try_get("name")?;
            if old_group == group && old_name == name {
                return Ok(());
            }
            bail!("Leave {old_group} as {old_name} before joining another group or name.");
        }
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM members WHERE group_name = ?")
            .bind(group)
            .fetch_one(&mut *transaction)
            .await?;
        if count >= MAX_MEMBERS {
            bail!("Group {group} is full ({MAX_MEMBERS} members).");
        }
        sqlx::query("INSERT INTO members(thread_id, group_name, name) VALUES (?, ?, ?)")
            .bind(thread_id.to_string())
            .bind(group)
            .bind(name)
            .execute(&mut *transaction)
            .await
            .with_context(|| format!("Name {name} is already taken in {group}"))?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn leave(&self, thread_id: ThreadId) -> Result<bool> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM messages WHERE recipient = ?")
            .bind(thread_id.to_string())
            .execute(&mut *transaction)
            .await?;
        let removed = sqlx::query("DELETE FROM members WHERE thread_id = ?")
            .bind(thread_id.to_string())
            .execute(&mut *transaction)
            .await?
            .rows_affected()
            > 0;
        transaction.commit().await?;
        Ok(removed)
    }

    pub async fn membership(&self, thread_id: ThreadId) -> Result<Option<Membership>> {
        let row = sqlx::query("SELECT group_name, name FROM members WHERE thread_id = ?")
            .bind(thread_id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(|row| {
            Ok(Membership {
                group: row.try_get("group_name")?,
                name: row.try_get("name")?,
                thread_id,
            })
        })
        .transpose()
    }

    pub async fn members(&self, thread_id: ThreadId) -> Result<Vec<MemberStatus>> {
        let Some(member) = self.membership(thread_id).await? else {
            return Ok(Vec::new());
        };
        let rows = sqlx::query(
            "SELECT name, owner IS NOT NULL AND seen_at >= unixepoch() - ? AS online
             FROM members WHERE group_name = ? ORDER BY name",
        )
        .bind(ONLINE_LEASE_SECONDS)
        .bind(member.group)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(MemberStatus {
                    name: row.try_get("name")?,
                    online: row.try_get::<i64, _>("online")? != 0,
                })
            })
            .collect()
    }

    pub async fn mark_online(&self, thread_id: ThreadId, owner: &str) -> Result<()> {
        sqlx::query("UPDATE members SET owner = ?, seen_at = unixepoch() WHERE thread_id = ?")
            .bind(owner)
            .bind(thread_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn heartbeat(&self, thread_id: ThreadId, owner: &str) -> Result<bool> {
        Ok(sqlx::query(
            "UPDATE members SET seen_at = unixepoch() WHERE thread_id = ? AND owner = ?",
        )
        .bind(thread_id.to_string())
        .bind(owner)
        .execute(&self.pool)
        .await?
        .rows_affected()
            > 0)
    }

    pub async fn mark_offline(&self, thread_id: ThreadId, owner: &str) -> Result<()> {
        sqlx::query(
            "UPDATE members SET owner = NULL, seen_at = NULL WHERE thread_id = ? AND owner = ?",
        )
        .bind(thread_id.to_string())
        .bind(owner)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn send(
        &self,
        sender_id: ThreadId,
        recipients: Option<&[String]>,
        body: &str,
        priority: Priority,
    ) -> Result<SendReceipt> {
        if body.is_empty() || body.len() > MAX_MESSAGE_BYTES {
            bail!("Message must contain 1–{MAX_MESSAGE_BYTES} UTF-8 bytes; nothing was sent.");
        }
        let mut transaction = self.pool.begin().await?;
        let sender = sqlx::query("SELECT group_name, name FROM members WHERE thread_id = ?")
            .bind(sender_id.to_string())
            .fetch_optional(&mut *transaction)
            .await?
            .context("This session is not in a group")?;
        let group: String = sender.try_get("group_name")?;
        let sender_name: String = sender.try_get("name")?;
        let rows = sqlx::query(
            "SELECT thread_id, name, owner IS NOT NULL AND seen_at >= unixepoch() - ? AS online
             FROM members WHERE group_name = ? AND thread_id != ? ORDER BY name",
        )
        .bind(ONLINE_LEASE_SECONDS)
        .bind(&group)
        .bind(sender_id.to_string())
        .fetch_all(&mut *transaction)
        .await?;
        let mut targets = Vec::new();
        let mut requested = HashSet::new();
        if let Some(names) = recipients {
            if names.is_empty() || names.len() > MAX_MEMBERS as usize {
                bail!("Provide between 1 and {MAX_MEMBERS} recipient names.");
            }
            for name in names {
                validate_label(name)?;
                if !requested.insert(name.as_str()) {
                    bail!("Recipient {name} appears twice; nothing was sent.");
                }
            }
        }
        for row in rows {
            let name: String = row.try_get("name")?;
            if recipients.is_none_or(|_| requested.remove(name.as_str())) {
                targets.push((
                    row.try_get::<String, _>("thread_id")?,
                    MemberStatus {
                        name,
                        online: row.try_get::<i64, _>("online")? != 0,
                    },
                ));
            }
        }
        if !requested.is_empty() {
            bail!(
                "Unknown peer(s): {}. Nothing was sent.",
                requested.into_iter().collect::<Vec<_>>().join(", ")
            );
        }
        if targets.is_empty() {
            bail!("No other members in {group}; nothing was sent.");
        }
        for (thread_id, _) in &targets {
            let (pending_bytes, pending_count): (i64, i64) = sqlx::query_as(
                "SELECT COALESCE(SUM(length(CAST(body AS BLOB))), 0), COUNT(*) FROM messages WHERE recipient = ?",
            )
            .bind(thread_id)
            .fetch_one(&mut *transaction)
            .await?;
            let limit = if priority == Priority::High {
                MAX_PENDING_BYTES
            } else {
                LOW_PENDING_BYTES
            };
            let count_limit = if priority == Priority::High {
                MAX_PENDING_MESSAGES
            } else {
                MAX_PENDING_MESSAGES - 1
            };
            if pending_bytes + body.len() as i64 > limit || pending_count >= count_limit {
                bail!(
                    "A peer's pending mail is full; nothing was sent. Try again after they catch up."
                );
            }
        }
        for (thread_id, _) in &targets {
            sqlx::query("INSERT INTO messages(recipient, sender, body, high) VALUES (?, ?, ?, ?)")
                .bind(thread_id)
                .bind(&sender_name)
                .bind(body)
                .bind(priority == Priority::High)
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        Ok(SendReceipt {
            recipients: targets.into_iter().map(|(_, status)| status).collect(),
        })
    }

    pub async fn pending(&self, thread_id: ThreadId, owner: &str) -> Result<Vec<PendingMessage>> {
        let rows = sqlx::query(
            "SELECT m.id, m.sender, m.body, m.high FROM messages m
             JOIN members r ON r.thread_id = m.recipient
             WHERE m.recipient = ? AND r.owner = ? ORDER BY m.id",
        )
        .bind(thread_id.to_string())
        .bind(owner)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(PendingMessage {
                    id: row.try_get("id")?,
                    sender: row.try_get("sender")?,
                    body: row.try_get("body")?,
                    high: row.try_get::<i64, _>("high")? != 0,
                })
            })
            .collect()
    }

    pub async fn acknowledge(&self, thread_id: ThreadId, owner: &str, ids: &[i64]) -> Result<()> {
        for id in ids {
            sqlx::query(
                "DELETE FROM messages WHERE id = ? AND recipient = ?
                 AND EXISTS (SELECT 1 FROM members WHERE thread_id = ? AND owner = ?)",
            )
            .bind(id)
            .bind(thread_id.to_string())
            .bind(thread_id.to_string())
            .bind(owner)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }
}

fn validate_label(label: &str) -> Result<()> {
    if label.is_empty()
        || label.len() > 32
        || !label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        bail!("Names must be 1–32 ASCII letters, digits, hyphens, or underscores.");
    }
    Ok(())
}
