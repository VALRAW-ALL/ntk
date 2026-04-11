use crate::detector::OutputType;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{sqlite::SqlitePool, Row};

// ---------------------------------------------------------------------------
// Record — one compression event
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct CompressionRecord {
    pub command: String,
    pub output_type: OutputType,
    pub original_tokens: usize,
    pub compressed_tokens: usize,
    /// Which layer produced the final output: 1, 2, or 3.
    pub layer_used: u8,
    pub latency_ms: u64,
    pub rtk_pre_filtered: bool,
    pub timestamp: DateTime<Utc>,
}

impl CompressionRecord {
    /// Compression ratio: 0.0 = no compression, 1.0 = 100% reduction.
    pub fn ratio(&self) -> f32 {
        if self.original_tokens == 0 {
            return 0.0;
        }
        let saved = self.original_tokens.saturating_sub(self.compressed_tokens);
        saved as f32 / self.original_tokens as f32
    }
}

// ---------------------------------------------------------------------------
// SessionSummary — aggregate view returned by GET /metrics
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct SessionSummary {
    pub total_compressions: usize,
    pub total_tokens_saved: usize,
    pub average_ratio: f32,
    /// Distribution: how many compressions used each layer.
    pub layer_counts: [usize; 3],
    pub rtk_pre_filtered_count: usize,
}

// ---------------------------------------------------------------------------
// MetricsStore
// ---------------------------------------------------------------------------

pub struct MetricsStore {
    records: Vec<CompressionRecord>,
}

impl MetricsStore {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
        }
    }

    /// Append a new compression record.
    pub fn record(&mut self, r: CompressionRecord) {
        self.records.push(r);
    }

    /// Return the last `n` records (most-recent-last slice).
    pub fn recent(&self, n: usize) -> &[CompressionRecord] {
        let len = self.records.len();
        if n >= len {
            &self.records
        } else {
            &self.records[len.saturating_sub(n)..]
        }
    }

    /// Compute aggregate statistics across all stored records.
    pub fn session_summary(&self) -> SessionSummary {
        if self.records.is_empty() {
            return SessionSummary {
                total_compressions: 0,
                total_tokens_saved: 0,
                average_ratio: 0.0,
                layer_counts: [0; 3],
                rtk_pre_filtered_count: 0,
            };
        }

        let mut total_tokens_saved = 0usize;
        let mut ratio_sum = 0.0f32;
        let mut layer_counts = [0usize; 3];
        let mut rtk_pre_filtered_count = 0usize;

        for r in &self.records {
            total_tokens_saved = total_tokens_saved
                .saturating_add(r.original_tokens.saturating_sub(r.compressed_tokens));
            ratio_sum += r.ratio();

            // layer_used is 1-indexed; clamp to valid range.
            let idx = (r.layer_used as usize).saturating_sub(1).min(2);
            layer_counts[idx] = layer_counts[idx].saturating_add(1);

            if r.rtk_pre_filtered {
                rtk_pre_filtered_count = rtk_pre_filtered_count.saturating_add(1);
            }
        }

        let total = self.records.len();
        let average_ratio = ratio_sum / total as f32;

        SessionSummary {
            total_compressions: total,
            total_tokens_saved,
            average_ratio,
            layer_counts,
            rtk_pre_filtered_count,
        }
    }
}

impl Default for MetricsStore {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// MetricsDb — async SQLite persistence (optional, alongside in-memory store)
// ---------------------------------------------------------------------------

pub struct MetricsDb {
    pool: SqlitePool,
}

impl MetricsDb {
    /// Initialize the SQLite database at `path`, creating tables if needed.
    pub async fn init(path: &std::path::Path) -> Result<Self> {
        // Ensure parent directory exists.
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating db directory {}", parent.display()))?;
        }

        let url = format!("sqlite://{}?mode=rwc", path.display());
        let pool = SqlitePool::connect(&url)
            .await
            .with_context(|| format!("opening SQLite db at {}", path.display()))?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS compression_records (
                id               INTEGER PRIMARY KEY AUTOINCREMENT,
                command          TEXT    NOT NULL,
                output_type      TEXT    NOT NULL,
                original_tokens  INTEGER NOT NULL,
                compressed_tokens INTEGER NOT NULL,
                layer_used       INTEGER NOT NULL,
                latency_ms       INTEGER NOT NULL,
                rtk_pre_filtered BOOLEAN NOT NULL DEFAULT 0,
                created_at       DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
        )
        .execute(&pool)
        .await
        .context("creating compression_records table")?;

        Ok(Self { pool })
    }

    /// Persist a compression record to SQLite.
    pub async fn persist(&self, record: &CompressionRecord) -> Result<()> {
        let output_type = format!("{:?}", record.output_type).to_lowercase();
        sqlx::query(
            "INSERT INTO compression_records
                (command, output_type, original_tokens, compressed_tokens,
                 layer_used, latency_ms, rtk_pre_filtered)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&record.command)
        .bind(&output_type)
        .bind(record.original_tokens as i64)
        .bind(record.compressed_tokens as i64)
        .bind(record.layer_used as i64)
        .bind(record.latency_ms as i64)
        .bind(record.rtk_pre_filtered)
        .execute(&self.pool)
        .await
        .context("inserting compression record")?;
        Ok(())
    }

    /// Return the last `n` records, most-recent-last.
    pub async fn history(&self, n: usize) -> Result<Vec<HistoryRow>> {
        let rows = sqlx::query(
            "SELECT command, output_type, original_tokens, compressed_tokens,
                    layer_used, latency_ms, created_at
             FROM compression_records
             ORDER BY id DESC LIMIT ?",
        )
        .bind(n as i64)
        .fetch_all(&self.pool)
        .await
        .context("fetching history")?;

        let mut result: Vec<HistoryRow> = rows
            .iter()
            .map(|r| HistoryRow {
                command: r.get::<String, _>("command"),
                output_type: r.get::<String, _>("output_type"),
                original_tokens: r.get::<i64, _>("original_tokens") as usize,
                compressed_tokens: r.get::<i64, _>("compressed_tokens") as usize,
                layer_used: r.get::<i64, _>("layer_used") as u8,
                latency_ms: r.get::<i64, _>("latency_ms") as u64,
                created_at: r.get::<String, _>("created_at"),
            })
            .collect();
        result.reverse(); // oldest first
        Ok(result)
    }

    /// Return aggregate summary for the past `history_days` days.
    pub async fn summary(&self, history_days: u32) -> Result<PersistentSummary> {
        let row = sqlx::query(
            "SELECT COUNT(*) as total,
                    COALESCE(SUM(original_tokens - compressed_tokens), 0) as saved,
                    COALESCE(AVG(CAST(original_tokens - compressed_tokens AS REAL) / NULLIF(original_tokens, 0)), 0) as avg_ratio
             FROM compression_records
             WHERE created_at >= datetime('now', ?)",
        )
        .bind(format!("-{history_days} days"))
        .fetch_one(&self.pool)
        .await
        .context("fetching summary")?;

        Ok(PersistentSummary {
            total_compressions: row.get::<i64, _>("total") as usize,
            total_tokens_saved: row.get::<i64, _>("saved") as usize,
            average_ratio: row.get::<f64, _>("avg_ratio") as f32,
        })
    }
}

/// A single row from the history query.
#[derive(Debug, Clone, Serialize)]
pub struct HistoryRow {
    pub command: String,
    pub output_type: String,
    pub original_tokens: usize,
    pub compressed_tokens: usize,
    pub layer_used: u8,
    pub latency_ms: u64,
    pub created_at: String,
}

/// Aggregate summary over a date range (from SQLite).
#[derive(Debug, Clone, Serialize)]
pub struct PersistentSummary {
    pub total_compressions: usize,
    pub total_tokens_saved: usize,
    pub average_ratio: f32,
}

// ---------------------------------------------------------------------------
// Serialize OutputType (needed for CompressionRecord JSON)
// ---------------------------------------------------------------------------

impl Serialize for OutputType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let s = match self {
            OutputType::Test => "test",
            OutputType::Build => "build",
            OutputType::Log => "log",
            OutputType::Diff => "diff",
            OutputType::Generic => "generic",
        };
        serializer.serialize_str(s)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_record(original: usize, compressed: usize, layer: u8, rtk: bool) -> CompressionRecord {
        CompressionRecord {
            command: "cargo test".to_owned(),
            output_type: OutputType::Test,
            original_tokens: original,
            compressed_tokens: compressed,
            layer_used: layer,
            latency_ms: 5,
            rtk_pre_filtered: rtk,
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn test_empty_store_summary() {
        let store = MetricsStore::new();
        let summary = store.session_summary();
        assert_eq!(summary.total_compressions, 0);
        assert_eq!(summary.total_tokens_saved, 0);
        assert_eq!(summary.average_ratio, 0.0);
    }

    #[test]
    fn test_record_and_summary() {
        let mut store = MetricsStore::new();
        store.record(make_record(1000, 200, 2, false));
        store.record(make_record(500, 100, 1, true));

        let s = store.session_summary();
        assert_eq!(s.total_compressions, 2);
        assert_eq!(s.total_tokens_saved, 1200);
        assert_eq!(s.rtk_pre_filtered_count, 1);
        assert_eq!(s.layer_counts[0], 1); // layer 1
        assert_eq!(s.layer_counts[1], 1); // layer 2
    }

    #[test]
    fn test_ratio_calculation() {
        let r = make_record(1000, 200, 2, false);
        assert!(
            (r.ratio() - 0.8).abs() < 0.001,
            "expected ~0.8, got {}",
            r.ratio()
        );
    }

    #[test]
    fn test_recent_returns_last_n() {
        let mut store = MetricsStore::new();
        for i in 0..10u64 {
            let mut r = make_record(100, 50, 1, false);
            r.latency_ms = i;
            store.record(r);
        }
        let recent = store.recent(3);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[2].latency_ms, 9);
    }
}
