use crate::error::Result;
use crate::pool::DbPool;
use teramind_core::ids::SessionId;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Clone)]
pub struct SearchRepo {
    pool: DbPool,
}

#[derive(Debug, Clone)]
pub struct RankedTurn {
    pub turn_id: Uuid,
    pub session_id: Uuid,
    pub ordinal: i32,
    pub ts: OffsetDateTime,
    pub project_id: Option<Uuid>,
    pub fts_score: f32,
    pub trgm_score: f32,
    pub user_prompt: Option<String>,
    pub assistant_text: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RankedDiff {
    pub diff_id: Uuid,
    pub session_id: Uuid,
    pub rel_path: String,
    pub ts: OffsetDateTime,
    pub project_id: Option<Uuid>,
    pub trgm_score: f32,
    pub pre_excerpt: String,
    pub post_excerpt: String,
}

#[derive(Debug, Clone)]
pub struct RankedSkill {
    pub skill_id: Uuid,
    pub name: String,
    pub body: String,
    pub trgm_score: f32,
}

impl SearchRepo {
    pub fn new(pool: DbPool) -> Self { Self { pool } }

    pub async fn fts_turns(&self, query: &str, limit: u32) -> Result<Vec<RankedTurn>> {
        let rows: Vec<(Uuid, Uuid, i32, OffsetDateTime, Option<Uuid>, f32, Option<String>, Option<String>)> = sqlx::query_as(
            r#"
            SELECT
                f.turn_id, f.session_id, f.ordinal, f.ts,
                s.project_id,
                ts_rank_cd(f.document, plainto_tsquery('english', $1))::float4 AS fts_score,
                t.user_prompt, t.assistant_text
            FROM traces_fts f
            JOIN turns    t ON t.id = f.turn_id
            JOIN sessions s ON s.id = f.session_id
            WHERE f.document @@ plainto_tsquery('english', $1)
            ORDER BY fts_score DESC
            LIMIT $2
            "#,
        )
        .bind(query)
        .bind(limit as i64)
        .fetch_all(self.pool.pg()).await?;

        Ok(rows.into_iter().map(|(turn_id, session_id, ordinal, ts, project_id, fts, prompt, text)| {
            RankedTurn { turn_id, session_id, ordinal, ts, project_id, fts_score: fts, trgm_score: 0.0,
                         user_prompt: prompt, assistant_text: text }
        }).collect())
    }

    pub async fn trgm_diffs(&self, query: &str, limit: u32) -> Result<Vec<RankedDiff>> {
        let rows: Vec<(Uuid, Uuid, String, OffsetDateTime, Option<Uuid>, f32, String, String)> = sqlx::query_as(
            r#"
            SELECT
                fd.id, fd.session_id, fd.rel_path, fd.captured_at,
                s.project_id,
                GREATEST(similarity(fd.pre_excerpt, $1), similarity(fd.post_excerpt, $1))::float4 AS trgm_score,
                fd.pre_excerpt, fd.post_excerpt
            FROM file_diffs fd
            JOIN sessions s ON s.id = fd.session_id
            WHERE fd.pre_excerpt % $1 OR fd.post_excerpt % $1
            ORDER BY trgm_score DESC
            LIMIT $2
            "#,
        )
        .bind(query)
        .bind(limit as i64)
        .fetch_all(self.pool.pg()).await?;

        Ok(rows.into_iter().map(|(diff_id, session_id, rel_path, ts, project_id, trgm, pre, post)| {
            RankedDiff { diff_id, session_id, rel_path, ts, project_id, trgm_score: trgm, pre_excerpt: pre, post_excerpt: post }
        }).collect())
    }

    pub async fn trgm_skills(&self, query: &str, limit: u32) -> Result<Vec<RankedSkill>> {
        let rows: Vec<(Uuid, String, String, f32)> = sqlx::query_as(
            r#"
            SELECT id, name, body,
                   GREATEST(similarity(name, $1), similarity(body, $1))::float4 AS trgm_score
            FROM skills
            WHERE name % $1 OR body % $1
            ORDER BY trgm_score DESC
            LIMIT $2
            "#,
        )
        .bind(query)
        .bind(limit as i64)
        .fetch_all(self.pool.pg()).await?;

        Ok(rows.into_iter().map(|(skill_id, name, body, trgm)| {
            RankedSkill { skill_id, name, body, trgm_score: trgm }
        }).collect())
    }

    pub async fn recent_turns_in_project(&self, project_id: Option<Uuid>, cwd: &str, limit: u32) -> Result<Vec<RankedTurn>> {
        let rows: Vec<(Uuid, Uuid, i32, OffsetDateTime, Option<Uuid>, Option<String>, Option<String>)> = match project_id {
            Some(pid) => sqlx::query_as(
                r#"
                SELECT t.id, t.session_id, t.ordinal, t.started_at, s.project_id, t.user_prompt, t.assistant_text
                FROM turns t
                JOIN sessions s ON s.id = t.session_id
                WHERE s.project_id = $1
                ORDER BY t.started_at DESC
                LIMIT $2
                "#,
            ).bind(pid).bind(limit as i64).fetch_all(self.pool.pg()).await?,
            None => sqlx::query_as(
                r#"
                SELECT t.id, t.session_id, t.ordinal, t.started_at, s.project_id, t.user_prompt, t.assistant_text
                FROM turns t
                JOIN sessions s ON s.id = t.session_id
                WHERE s.cwd = $1
                ORDER BY t.started_at DESC
                LIMIT $2
                "#,
            ).bind(cwd).bind(limit as i64).fetch_all(self.pool.pg()).await?,
        };
        Ok(rows.into_iter().map(|(turn_id, session_id, ordinal, ts, project_id, prompt, text)| {
            RankedTurn { turn_id, session_id, ordinal, ts, project_id, fts_score: 0.0, trgm_score: 0.0,
                         user_prompt: prompt, assistant_text: text }
        }).collect())
    }

    pub async fn upsert_skill(&self, req: &teramind_core::types::SaveSkillRequest) -> Result<teramind_core::types::SkillRef> {
        let row: (Uuid, String) = sqlx::query_as(
            r#"
            INSERT INTO skills (name, description, body, source, source_session_ids)
            VALUES ($1, $2, $3, 'authored', $4)
            ON CONFLICT (name) DO UPDATE SET
              description = EXCLUDED.description,
              body        = EXCLUDED.body,
              source_session_ids = EXCLUDED.source_session_ids,
              updated_at  = now()
            RETURNING id, name
            "#,
        )
        .bind(&req.name)
        .bind(&req.description)
        .bind(&req.body)
        .bind(req.source_session_ids.iter().map(|s| s.0).collect::<Vec<Uuid>>())
        .fetch_one(self.pool.pg()).await?;
        Ok(teramind_core::types::SkillRef {
            id: teramind_core::ids::SkillId(row.0),
            name: row.1,
        })
    }
}
