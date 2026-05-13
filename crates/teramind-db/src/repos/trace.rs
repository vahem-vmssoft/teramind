use crate::error::Result;
use crate::pool::DbPool;
use teramind_core::ids::{SessionId, ToolCallId, TurnId};
use time::OffsetDateTime;

#[derive(Clone)]
pub struct TraceRepo {
    pool: DbPool,
}

impl TraceRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn upsert_turn(
        &self,
        session_id: SessionId,
        ordinal: i32,
        started_at: OffsetDateTime,
        user_prompt: Option<&str>,
    ) -> Result<TurnId> {
        let r: (uuid::Uuid,) = sqlx::query_as(
            r#"
            INSERT INTO turns (session_id, ordinal, started_at, user_prompt)
            VALUES ($1,$2,$3,$4)
            ON CONFLICT (session_id, ordinal) DO UPDATE SET user_prompt = COALESCE(EXCLUDED.user_prompt, turns.user_prompt)
            RETURNING id
            "#)
            .bind(session_id.0).bind(ordinal).bind(started_at).bind(user_prompt)
            .fetch_one(self.pool.pg()).await?;
        Ok(TurnId(r.0))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn finalize_turn(
        &self,
        id: TurnId,
        ended_at: OffsetDateTime,
        assistant_text: Option<&str>,
        thinking: Option<&str>,
        model: Option<&str>,
        input_tokens: Option<i32>,
        output_tokens: Option<i32>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE turns SET ended_at=$1, assistant_text=$2, thinking=$3, model=$4,
                             input_tokens=$5, output_tokens=$6
            WHERE id=$7
            "#,
        )
        .bind(ended_at)
        .bind(assistant_text)
        .bind(thinking)
        .bind(model)
        .bind(input_tokens)
        .bind(output_tokens)
        .bind(id.0)
        .execute(self.pool.pg())
        .await?;
        Ok(())
    }

    pub async fn insert_tool_call_start(
        &self,
        turn_id: TurnId,
        ordinal: i32,
        name: &str,
        input: &serde_json::Value,
        started_at: OffsetDateTime,
    ) -> Result<ToolCallId> {
        let r: (uuid::Uuid,) = sqlx::query_as(
            r#"
            INSERT INTO tool_calls (turn_id, ordinal, name, input, started_at)
            VALUES ($1,$2,$3,$4,$5)
            ON CONFLICT (turn_id, ordinal) DO UPDATE SET name = EXCLUDED.name
            RETURNING id
            "#,
        )
        .bind(turn_id.0)
        .bind(ordinal)
        .bind(name)
        .bind(input)
        .bind(started_at)
        .fetch_one(self.pool.pg())
        .await?;
        Ok(ToolCallId(r.0))
    }

    pub async fn insert_tool_call_start_with_id(
        &self,
        id: ToolCallId,
        turn_id: TurnId,
        ordinal: i32,
        name: &str,
        input: &serde_json::Value,
        started_at: OffsetDateTime,
    ) -> Result<ToolCallId> {
        sqlx::query(
            r#"
            INSERT INTO tool_calls (id, turn_id, ordinal, name, input, started_at)
            VALUES ($1,$2,$3,$4,$5,$6)
            ON CONFLICT (turn_id, ordinal) DO NOTHING
            "#)
            .bind(id.0).bind(turn_id.0).bind(ordinal).bind(name).bind(input).bind(started_at)
            .execute(self.pool.pg()).await?;
        Ok(id)
    }

    pub async fn finalize_tool_call(
        &self,
        id: ToolCallId,
        output: &str,
        is_error: bool,
        duration_ms: i32,
    ) -> Result<()> {
        sqlx::query("UPDATE tool_calls SET output=$1, is_error=$2, duration_ms=$3 WHERE id=$4")
            .bind(output)
            .bind(is_error)
            .bind(duration_ms)
            .bind(id.0)
            .execute(self.pool.pg())
            .await?;
        Ok(())
    }
}
