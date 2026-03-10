use anyhow::Result;
use tokio_rusqlite::Connection;

use super::models::PushSubscription;

pub async fn find_all_notification_subscriptions(
    db: &Connection,
) -> Result<Vec<PushSubscription>> {
    let subscriptions = db.call(|conn| {
        let mut stmt =
            conn.prepare("SELECT endpoint, p256dh, auth FROM push_subscription WHERE is_valid = 1")?;
        let rows = stmt
            .query_map([], |i| {
                Ok(PushSubscription {
                    endpoint: i.get(0)?,
                    p256dh: i.get(1)?,
                    auth: i.get(2)?,
                })
            })?
            .filter_map(Result::ok)
            .collect::<Vec<PushSubscription>>();
        Ok(rows)
    });
    Ok(subscriptions.await?)
}

pub async fn mark_push_subscription_invalid(
    db: &Connection,
    endpoint: &str,
) -> Result<()> {
    let endpoint = endpoint.to_string();
    db.call(move |conn| {
        conn.execute(
            "UPDATE push_subscription SET is_valid = 0 WHERE endpoint = ?1",
            [&endpoint],
        )?;
        Ok(())
    })
    .await
    .map_err(anyhow::Error::new)
}
