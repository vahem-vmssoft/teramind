use crate::error::Result;
use crate::pool::DbPool;
use teramind_core::ids::ProjectId;
use teramind_core::types::Project;
use time::OffsetDateTime;

#[derive(Clone)]
pub struct ProjectRepo {
    pool: DbPool,
}

impl ProjectRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn upsert_by_root(
        &self,
        root_path: &str,
        git_remote: Option<&str>,
        display_name: Option<&str>,
    ) -> Result<Project> {
        let r: (
            uuid::Uuid,
            String,
            Option<String>,
            Option<String>,
            OffsetDateTime,
        ) = sqlx::query_as(
            r#"
            INSERT INTO projects (root_path, git_remote, display_name)
            VALUES ($1, $2, $3)
            ON CONFLICT (root_path) DO UPDATE SET
                git_remote = COALESCE(EXCLUDED.git_remote, projects.git_remote),
                display_name = COALESCE(EXCLUDED.display_name, projects.display_name)
            RETURNING id, root_path, git_remote, display_name, first_seen
            "#,
        )
        .bind(root_path)
        .bind(git_remote)
        .bind(display_name)
        .fetch_one(self.pool.pg())
        .await?;
        Ok(Project {
            id: ProjectId(r.0),
            root_path: r.1,
            git_remote: r.2,
            display_name: r.3,
            first_seen: r.4,
        })
    }
}
