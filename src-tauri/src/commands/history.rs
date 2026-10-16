use super::*;

#[tauri::command]
pub async fn get_recent(
    app: AppHandle,
    limit: Option<usize>,
    offset: Option<usize>,
    before_id: Option<i64>,
    search: Option<String>,
    app_name: Option<String>,
) -> Result<Vec<db::RecentEntry>, String> {
    let limit = limit.unwrap_or(100);
    let offset = offset.unwrap_or(0);
    let search = search.filter(|s| !s.trim().is_empty());
    let app_name = app_name.filter(|s| !s.trim().is_empty());
    run_db(&app, "get_recent", move |db| {
        if before_id.is_some() || offset == 0 {
            db::query_recent_page_before(
                db,
                limit,
                before_id,
                search.as_deref(),
                app_name.as_deref(),
            )
        } else {
            db::query_recent_page(db, limit, offset, search.as_deref(), app_name.as_deref())
        }
    })
    .await
}

#[tauri::command]
pub async fn get_history_apps(app: AppHandle) -> Result<Vec<String>, String> {
    run_db(&app, "get_history_apps", db::query_distinct_apps).await
}

#[tauri::command]
pub async fn get_stats(app: AppHandle) -> Result<db::Stats, String> {
    run_db(&app, "get_stats", db::query_stats).await
}

#[tauri::command]
pub async fn get_insights(
    app: AppHandle,
    days: i64,
    context_id: Option<i64>,
) -> Result<db::Insights, String> {
    run_db(&app, "get_insights", move |db| {
        db::query_insights(db, days, context_id)
    })
    .await
}

#[tauri::command]
pub async fn get_insights_pricing(app: AppHandle) -> Result<db::PricingSnapshot, String> {
    let fresh = run_db(
        &app,
        "get_insights_pricing_freshness",
        db::pricing_cache_is_fresh,
    )
    .await?;

    if !fresh {
        match crate::api::openrouter::fetch_model_pricing().await {
            Ok(rates) => {
                let fetched_at = chrono::Utc::now().timestamp();
                run_db(&app, "save_insights_pricing", move |db| {
                    db::replace_pricing_cache(db, fetched_at, &rates)
                })
                .await?;
            }
            Err(error) => {
                log::warn!("insights: OpenRouter pricing refresh failed: {error}");
            }
        }
    }

    run_db(&app, "get_insights_pricing", db::query_pricing_snapshot).await
}

#[tauri::command]
pub async fn count_old_transcriptions(app: AppHandle, retention: String) -> Result<i64, String> {
    let Some(days) = store::history_retention_days(&retention) else {
        return Ok(0);
    };
    run_db(&app, "count_old_transcriptions", move |db| {
        db::count_transcriptions_older_than(db, days)
    })
    .await
}

#[tauri::command]
pub async fn retry_transcription(
    app: AppHandle,
    state: tauri::State<'_, SharedState>,
) -> Result<db::RecentEntry, String> {
    pipeline::retry_transcription_impl(&app, &state)
        .await
        .map_err(|e| crate::api::user_facing_error(&e))
}

#[tauri::command]
pub async fn clear_cleanup_cache(app: AppHandle) -> Result<usize, String> {
    run_db(&app, "clear_cleanup_cache", db::cleanup_cache_clear_all).await
}

#[tauri::command]
pub async fn get_cleanup_cache_status(app: AppHandle) -> Result<CleanupCacheStatus, String> {
    let (payload_bytes, entry_count) = run_db(&app, "get_cleanup_cache_status", move |db| {
        db::cleanup_cache_prune_expired(db)?;
        let payload_bytes = db::cleanup_cache_payload_bytes(db)?;
        let count = db::cleanup_cache_count(db)
            .map_err(|e| anyhow::anyhow!("Failed to count cleanup cache entries: {e}"))?;
        Ok((payload_bytes, count))
    })
    .await?;
    Ok(CleanupCacheStatus {
        entry_count,
        payload_bytes,
        session: crate::pipeline::cache::metrics(),
    })
}
