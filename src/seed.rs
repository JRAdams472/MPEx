use std::path::Path;

use anyhow::Context;
use sqlx::PgPool;

/// Executes every `*.sql` in `dir` in filename order after migrations.
/// Seed scripts must be idempotent (guard with WHERE NOT EXISTS / ON CONFLICT).
pub async fn run(pool: &PgPool, dir: &str) -> anyhow::Result<()> {
    let dir_path = Path::new(dir);
    if !dir_path.is_dir() {
        tracing::debug!(dir, "seed dir absent — skipping");
        return Ok(());
    }
    let mut files: Vec<_> = std::fs::read_dir(dir_path)
        .with_context(|| format!("read seed dir {dir}"))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "sql"))
        .collect();
    files.sort();

    for file in files {
        let sql =
            std::fs::read_to_string(&file).with_context(|| format!("read {}", file.display()))?;
        sqlx::raw_sql(&sql)
            .execute(pool)
            .await
            .with_context(|| format!("seed {}", file.display()))?;
        tracing::info!(file = %file.display(), "seeded");
    }
    Ok(())
}
