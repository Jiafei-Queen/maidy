//! 已停用语音偏好表的历史 migration。
//!
//! 新版本不再读写 `voice_preferences`，但迁移会记录在 `app_sqlite_migrations`。
//! 如果从统一 migration 列表删除该常量，已有数据库会被判定为“由更新版本迁移”，
//! 导致升级后无法启动；因此这里只保留 schema 与说明，不承载任何业务读写。

use crate::storage::database::SqliteMigration;

pub const VOICE_PREFERENCE_SCHEMA_V1: SqliteMigration = SqliteMigration {
    name: "voice_preference_schema_v1",
    sql: "CREATE TABLE IF NOT EXISTS voice_preferences (
            platform TEXT NOT NULL,
            account_id TEXT NOT NULL,
            target_type TEXT NOT NULL CHECK (target_type IN ('private', 'group')),
            target_id TEXT NOT NULL,
            enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
            updated_at TEXT NOT NULL,
            PRIMARY KEY (platform, account_id, target_type, target_id)
          );",
};
